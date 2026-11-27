//! Helpers CDP : filtre de selection de cible et facade paresseuse `LazyCdp`.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use adapter_cdp_webview::{CdpClient, TargetFilter};
use deveye_core::{DomTree, InteractionCommand, Screenshot, Size, StateDump, WindowInfo};
use deveye_ports::{
    AppInspector, InteractionSink, JsEvaluator, PortError, PortResult, Renderer, WindowLister,
};

use crate::cli::{Args, TargetMode};

/// Construit le filtre de selection de la page CDP cible a partir des arguments
/// CLI. Sans flag de filtre, le filtre est ouvert (premiere page qui n'est pas
/// about:blank, ADR-0014) ; --cdp-accept-blank (false par defaut) autorise en
/// plus la page about:blank transitoire d'une webview au boot.
pub(crate) fn build_cdp_filter(args: &Args) -> TargetFilter {
    TargetFilter {
        url_contains: args.cdp_target_filter_url.clone(),
        title_contains: args.cdp_target_filter_title.clone(),
        accept_blank: args.cdp_accept_blank,
    }
}

/// Tente la connexion initiale au CDP. Renvoie `None` si la cible n'utilise pas
/// de navigateur (`sdk`, `kwin-eis`, `x11`), ou si le CDP est injoignable au demarrage
/// (erreur non fatale — la reconnexion se fera au premier appel d'outil).
pub(crate) async fn try_initial_cdp(
    url: &str,
    filter: TargetFilter,
    target: TargetMode,
) -> Option<Arc<CdpClient>> {
    if !target.uses_cdp() {
        return None;
    }
    match CdpClient::connect(url, filter).await {
        Ok(client) => Some(Arc::new(client)),
        Err(e) => {
            warn!(
                erreur = %e,
                "CDP injoignable au démarrage — reconnexion automatique au premier appel d'outil"
            );
            None
        }
    }
}

/// Facade paresseuse sur le CDP : (re)connecte **a la demande** au lieu
/// d'exiger le navigateur au demarrage. Survit aussi a sa fermeture en cours
/// de session (relance de `just ui-eyes`) : un client dont le WebSocket est
/// mort (`is_closed`) est jete et remplace au prochain appel d'outil.
pub(crate) struct LazyCdp {
    url: String,
    filter: TargetFilter,
    slot: tokio::sync::Mutex<Option<Arc<CdpClient>>>,
}

impl LazyCdp {
    pub(crate) fn new(url: String, filter: TargetFilter, initial: Option<Arc<CdpClient>>) -> Self {
        Self {
            url,
            filter,
            slot: tokio::sync::Mutex::new(initial),
        }
    }

    /// Le client courant s'il est vivant, sinon une (re)connexion. L'echec
    /// est une erreur d'outil ordinaire — jamais la mort du serveur MCP.
    async fn client(&self) -> PortResult<Arc<CdpClient>> {
        // Le lock 'slot' est tenu pendant toute la (re)connexion ci-dessous —
        // partagee par les 5 ports CDP (Renderer/InteractionSink/AppInspector/
        // JsEvaluator/WindowLister). Choix assume : ca evite que deux outils
        // appeles en parallele pendant un trou de connexion ne declenchent
        // chacun leur propre CdpClient::connect (double WS, double attach).
        // Ce n'est sain qu'a partir du correctif A2 (client.rs) : chaque
        // etape de connect() est desormais bornee — GET /json (10 s), upgrade
        // WS (10 s), Target.setDiscoverTargets (CDP_RESPONSE_TIMEOUT, 20 s),
        // attente d'attach (5 s) — soit ~45 s au PIRE cas cumule, au lieu de
        // pouvoir bloquer indefiniment. Long mais fini : le gate MCP finit
        // toujours par etre libere avec une erreur actionnable.
        let mut slot = self.slot.lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
            warn!(url = %self.url, "client CDP mort — reconnexion en cours");
        }
        match CdpClient::connect(&self.url, self.filter.clone()).await {
            Ok(client) => {
                let client = Arc::new(client);
                *slot = Some(client.clone());
                Ok(client)
            }
            Err(e) => Err(PortError::Network(format!(
                "CDP injoignable sur {} ({e}) — lancer le navigateur cible \
                 (`just ui-eyes`) puis rappeler l'outil",
                self.url
            ))),
        }
    }
}

#[async_trait]
impl Renderer for LazyCdp {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        self.client().await?.screenshot().await
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        self.client().await?.viewport_size().await
    }
}

#[async_trait]
impl InteractionSink for LazyCdp {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        self.client().await?.dispatch(command).await
    }
}

#[async_trait]
impl AppInspector for LazyCdp {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        self.client().await?.dom_snapshot().await
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        self.client().await?.state_snapshot().await
    }
}

#[async_trait]
impl JsEvaluator for LazyCdp {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value> {
        self.client().await?.evaluate(expression).await
    }
}

#[async_trait]
impl WindowLister for LazyCdp {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        self.client().await?.list_windows().await
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use clap::Parser as _;

    fn filter(extra: &[&str]) -> TargetFilter {
        let args = Args::try_parse_from(["deveye-server"].into_iter().chain(extra.iter().copied()))
            .expect("arguments valides");
        build_cdp_filter(&args)
    }

    #[test]
    fn without_flags_the_filter_is_open_and_rejects_blank() {
        let f = filter(&[]);
        assert!(f.is_open());
        assert!(!f.accept_blank);
    }

    #[test]
    fn url_title_and_blank_flags_are_forwarded_independently() {
        let f = filter(&["--cdp-target-filter-url", "localhost"]);
        assert_eq!(f.url_contains.as_deref(), Some("localhost"));
        assert!(f.title_contains.is_none() && !f.accept_blank);

        let f = filter(&["--cdp-target-filter-title", "Accueil"]);
        assert_eq!(f.title_contains.as_deref(), Some("Accueil"));
        assert!(f.url_contains.is_none() && !f.accept_blank);

        let f = filter(&["--cdp-accept-blank"]);
        assert!(f.accept_blank && f.is_open());

        let f = filter(&[
            "--cdp-target-filter-url",
            "a",
            "--cdp-target-filter-title",
            "b",
            "--cdp-accept-blank",
        ]);
        assert!(f.url_contains.is_some() && f.title_contains.is_some() && f.accept_blank);
    }
}
