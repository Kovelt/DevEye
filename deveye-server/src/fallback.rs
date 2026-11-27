//! Logique de repli (mode `auto`) : structs `Fallback*` et macro `fallback_port!`.
//!
//! Chaque struct enroule un port `primary` et un port `fallback` : si le
//! primaire echoue avec `PortError::Network` (cible injoignable), l'appel est
//! delegue au repli ; toute autre erreur est propagee telle quelle.
//!
//! Isole de `orchestration.rs` (T5 audit post-M7) pour reduire la duplication
//! et faciliter les tests unitaires du repli sans I/O.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use deveye_core::{DomTree, InteractionCommand, Screenshot, Size, StateDump};
use deveye_ports::{AppInspector, InteractionSink, JsEvaluator, PortError, PortResult, Renderer};

// ── Predicat de repli ─────────────────────────────────────────────────────────

/// Vrai si l'erreur signale un probleme de **liaison** (cible injoignable)
/// plutot qu'un echec de l'operation elle-meme — le seul cas ou le mode
/// `auto` bascule sur le pont natif.
pub(crate) fn is_liaison_error(e: &PortError) -> bool {
    matches!(e, PortError::Network(_))
}

// ── Etat observable du repli ──────────────────────────────────────────────────

const UNDETERMINED: u8 = 0;
const PRIMARY: u8 = 1;
const FALLBACK: u8 = 2;

/// Backend qui sert réellement les appels du mode `auto`, partage par toutes
/// les facades de repli et lu par `target_info` (ADR-0034). Chaque **bascule**
/// est journalisée (pas chaque appel) : un repli silencieux faisait croire a
/// l'agent qu'il pilotait le webview alors que les gestes passaient par le pont
/// natif.
pub(crate) struct FallbackState {
    current: AtomicU8,
}

impl FallbackState {
    /// `primary_up` : le primaire était joignable au démarrage (sinon l'état
    /// reste indetermine jusqu'au premier appel).
    pub(crate) fn new(primary_up: bool) -> Self {
        Self {
            current: AtomicU8::new(if primary_up { PRIMARY } else { UNDETERMINED }),
        }
    }

    /// Le primaire a repondu (succès ou erreur d'une autre nature que la liaison).
    pub(crate) fn mark_primary(&self) {
        if self.current.swap(PRIMARY, Ordering::Relaxed) == FALLBACK {
            info!("CDP de nouveau joignable : les appels repassent par le primaire");
        }
    }

    /// Le primaire est injoignable : l'appel part sur le pont natif.
    pub(crate) fn mark_fallback(&self, cause: &PortError) {
        if self.current.swap(FALLBACK, Ordering::Relaxed) != FALLBACK {
            warn!(
                %cause,
                "repli sur le pont natif (SDK + OS) : le CDP est injoignable ; `target_info` \
                 indique le backend effectif"
            );
        }
    }

    /// Libellé expose par `target_info` : "cdp", "sdk" ou "indetermine".
    pub(crate) fn label(&self) -> &'static str {
        match self.current.load(Ordering::Relaxed) {
            PRIMARY => "cdp",
            FALLBACK => "sdk",
            _ => "indetermine",
        }
    }
}

// ── Macro de generation ───────────────────────────────────────────────────────

/// Genere une struct de repli nommee et son impl du trait donne.
///
/// Syntaxe :
/// ```text
/// fallback_port! {
///     /// doc
///     struct NomStruct : LePortTrait {
///         async fn methode(&self, arg: Type) -> PortResult<Foo> [arg.clone()] ;
///         async fn autre(&self) -> PortResult<Bar> [] ;
///     }
/// }
/// ```
///
/// Le bloc `[...]` contient les tokens passes tels quels au primaire :
/// - `[arg.clone()]` pour un argument possede (necessite `Clone`) ;
/// - `[arg]` pour une reference ou un type `Copy` (reutilisable sans clone) ;
/// - `[]` pour une methode sans arguments supplementaires.
///
/// Le repli recoit toujours les noms d'arguments d'origine (`$($arg),*`).
/// Cette separation evite tout appel `.clone()` superflu sur des types `Copy`
/// et reste clippy-clean sans `#[allow]`.
macro_rules! fallback_port {
    (
        $(#[$attr:meta])*
        struct $name:ident : $trait:path {
            $(
                async fn $method:ident (
                    &self $(, $arg:ident : $arg_ty:ty)*
                ) -> $ret:ty
                [ $($pexpr:tt)* ] ;
            )*
        }
    ) => {
        $(#[$attr])*
        pub(crate) struct $name {
            pub(crate) primary: Arc<dyn $trait>,
            pub(crate) fallback: Arc<dyn $trait>,
            pub(crate) state: Arc<FallbackState>,
        }

        #[async_trait]
        impl $trait for $name {
            $(
                async fn $method(&self $(, $arg: $arg_ty)*) -> $ret {
                    match self.primary.$method($($pexpr)*).await {
                        Err(e) if is_liaison_error(&e) => {
                            self.state.mark_fallback(&e);
                            self.fallback.$method($($arg),*).await
                        }
                        other => {
                            self.state.mark_primary();
                            other
                        }
                    }
                }
            )*
        }
    };
}

// ── Structs de repli ──────────────────────────────────────────────────────────

fallback_port! {
    /// Repli par appel pour les yeux : CDP d'abord, xcap si la liaison manque.
    struct FallbackRenderer : Renderer {
        async fn screenshot(&self) -> PortResult<Screenshot> [] ;
        async fn viewport_size(&self) -> PortResult<Size> [] ;
    }
}

fallback_port! {
    /// Repli par appel pour les mains : CDP d'abord, evenements JS via le SDK.
    struct FallbackInteraction : InteractionSink {
        async fn dispatch(&self, command: InteractionCommand) -> PortResult<()>
            [command.clone()] ;
    }
}

fallback_port! {
    /// Repli par appel pour le DOM : CDP d'abord, `outerHTML` via le SDK.
    struct FallbackDom : AppInspector {
        async fn dom_snapshot(&self) -> PortResult<DomTree> [] ;
        async fn state_snapshot(&self) -> PortResult<StateDump> [] ;
    }
}

fallback_port! {
    /// Repli par appel pour l'evaluateur JS : CDP d'abord, pompe SDK sinon.
    /// `expression` est une reference (`&str` est `Copy`) : on la passe
    /// directement au primaire sans `.clone()` superflu.
    struct FallbackEvaluator : JsEvaluator {
        async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value>
            [expression] ;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ----- H2 : predicat de repli -----

    /// `is_liaison_error` doit etre vrai uniquement pour `PortError::Network`.
    #[test]
    fn is_liaison_error_matches_only_network() {
        assert!(is_liaison_error(&PortError::Network(
            "conn refused".to_string()
        )));
        assert!(!is_liaison_error(&PortError::Timeout(
            "timed out".to_string()
        )));
        assert!(!is_liaison_error(&PortError::Render("bad".to_string())));
        assert!(!is_liaison_error(&PortError::Inspect("bad".to_string())));
        assert!(!is_liaison_error(&PortError::Protocol("bad".to_string())));
        assert!(!is_liaison_error(&PortError::Unsupported(
            "bad".to_string()
        )));
        assert!(!is_liaison_error(&PortError::Interaction(
            "bad".to_string()
        )));
        assert!(!is_liaison_error(&PortError::Log("bad".to_string())));
        assert!(!is_liaison_error(&PortError::Console("bad".to_string())));
        assert!(!is_liaison_error(&PortError::Route {
            route: "r".to_string(),
            message: "m".to_string(),
        }));
    }

    /// Un `FallbackEvaluator` dont le primaire echoue avec `Network` doit appeler
    /// le repli et renvoyer son resultat.
    #[tokio::test]
    async fn fallback_triggers_on_network_error_only() {
        struct NetworkFailEval;
        struct OkEval;

        #[async_trait]
        impl JsEvaluator for NetworkFailEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Err(PortError::Network("down".to_string()))
            }
        }

        #[async_trait]
        impl JsEvaluator for OkEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Ok(serde_json::json!(42))
            }
        }

        let state = Arc::new(FallbackState::new(false));
        assert_eq!(state.label(), "indetermine");
        let fe = FallbackEvaluator {
            primary: Arc::new(NetworkFailEval),
            fallback: Arc::new(OkEval),
            state: state.clone(),
        };
        let result = fe.evaluate("x").await.expect("le repli doit reussir");
        assert_eq!(result, serde_json::json!(42));
        // Le repli est observable : target_info dira "sdk".
        assert_eq!(state.label(), "sdk");
    }

    /// Un `FallbackEvaluator` dont le primaire echoue avec `Timeout` ne doit PAS
    /// appeler le repli : l'erreur est propagee telle quelle.
    #[tokio::test]
    async fn fallback_does_not_trigger_on_timeout() {
        struct TimeoutEval;
        struct TrackingEval(Arc<AtomicBool>);

        #[async_trait]
        impl JsEvaluator for TimeoutEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Err(PortError::Timeout("took too long".to_string()))
            }
        }

        #[async_trait]
        impl JsEvaluator for TrackingEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                self.0.store(true, Ordering::SeqCst);
                Ok(serde_json::json!(99))
            }
        }

        let called = Arc::new(AtomicBool::new(false));
        let state = Arc::new(FallbackState::new(false));
        let fe = FallbackEvaluator {
            primary: Arc::new(TimeoutEval),
            fallback: Arc::new(TrackingEval(called.clone())),
            state: state.clone(),
        };
        let err = fe
            .evaluate("x")
            .await
            .expect_err("le timeout doit se propager");
        assert!(
            matches!(err, PortError::Timeout(_)),
            "type d'erreur inattendu : {err:?}"
        );
        assert!(
            !called.load(Ordering::SeqCst),
            "le repli ne doit pas etre invoque sur Timeout"
        );
        // Une erreur non liée a la liaison prouve que le primaire répond.
        assert_eq!(state.label(), "cdp");
    }

    /// Le repli puis le retour du primaire se reflètent dans le libellé.
    #[test]
    fn state_follows_fallback_and_recovery() {
        let state = FallbackState::new(true);
        assert_eq!(state.label(), "cdp");
        state.mark_fallback(&PortError::Network("down".to_string()));
        assert_eq!(state.label(), "sdk");
        state.mark_fallback(&PortError::Network("down".to_string()));
        assert_eq!(state.label(), "sdk");
        state.mark_primary();
        assert_eq!(state.label(), "cdp");
    }
}
