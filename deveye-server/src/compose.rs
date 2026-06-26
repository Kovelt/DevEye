//! Cablage de composition : selectionne et assemble les adapters selon
//! `TargetMode` et le backend OS-level choisi.
//!
//! Point d'entree principal : [`build_mcp`].

use std::sync::Arc;

use adapter_cdp_webview::CdpClient;
use adapter_commands_http::SdkCommandRouter;
use adapter_composite::{
    CompositeInspector, CompositeWaitCondition, JsConsoleErrorSource, NoopConsoleErrorSource,
};
use adapter_dioxus_signals::{
    DioxusSignalsInspector, EvalDomInspector, JsInteraction, SdkEvaluator,
};
use adapter_interaction_kwin_eis::KwinEisInteraction;
use adapter_logs_tracing::SdkTracingLogSource;
use adapter_mcp_server::{DeveyeMcpServer, DeveyePorts};
use adapter_screencap_kwin::KwinRenderer;
use adapter_screencap_xcap::{MonitorRenderer, XcapRenderer};
use deveye_ports::{
    AppInspector, CommandRouter, ConsoleErrorSource, InteractionSink, JsEvaluator, LogSource,
    Renderer, WaitCondition, WindowLister,
};

use crate::cdp::{build_cdp_filter, try_initial_cdp, LazyCdp};
use crate::cli::{Args, ScreencapBackend, TargetMode};
use crate::fallback::{FallbackDom, FallbackEvaluator, FallbackInteraction, FallbackRenderer};
use crate::stubs::{EmptyWindowLister, UnsupportedEvaluator, UnsupportedInspector};

/// Le quintet pilotant : yeux, mains, DOM, évaluateur JS, liste des fenêtres.
type Pilotage = (
    Arc<dyn Renderer>,
    Arc<dyn InteractionSink>,
    Arc<dyn AppInspector>,
    Arc<dyn JsEvaluator>,
    Arc<dyn WindowLister>,
);

/// Vrai si la session est KDE Plasma sous Wayland natif (seul contexte où
/// `org.kde.KWin.ScreenShot2` répond).
fn is_kde_wayland() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);
    let kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false);
    wayland && kde
}

/// Sélectionne le renderer OS-level (yeux) selon le backend demandé. Le repli
/// vers xcap n'est jamais silencieux en cas d'erreur D-Bus : il est décidé ici,
/// au démarrage, par la nature de la session — pas par une erreur d'exécution.
///
/// Sans titre de fenêtre (`--screencap-window` absent), bascule sur le repli
/// **plein écran** [`MonitorRenderer`] — utile en CI headless (Xvfb) ou quand
/// la fenêtre cible n'est pas titrée.
fn build_os_renderer(title: Option<&str>, backend: ScreencapBackend) -> Arc<dyn Renderer> {
    let Some(title) = title else {
        eprintln!(
            "deveye-server : yeux = MonitorRenderer (plein écran ; --screencap-window absent)."
        );
        return Arc::new(MonitorRenderer);
    };
    let use_kwin = match backend {
        ScreencapBackend::Kwin => true,
        ScreencapBackend::Xcap => false,
        ScreencapBackend::Auto => is_kde_wayland(),
    };
    if use_kwin {
        eprintln!("deveye-server : yeux = KWin D-Bus ScreenShot2 (Wayland natif KDE).");
        Arc::new(KwinRenderer::new(title))
    } else {
        eprintln!("deveye-server : yeux = xcap (X11 / XWayland / Wayland non-KDE).");
        Arc::new(XcapRenderer::new(title))
    }
}

/// Tente de resoudre l'origine ecran `(x, y)` de la fenetre cible via KWin
/// Scripting, au demarrage du mode `kwin-eis`. Utilise [`KwinRenderer`] pour
/// rejouer le meme script KWin que pour la capture (sans dependance croisee
/// entre adapters : tout passe par `deveye-server`).
///
/// Repli sur `(0, 0)` si :
/// - `title` est absent (`--screencap-window` non fourni) ;
/// - le backend effectif n'est pas KWin (xcap ou session non-KDE/Wayland) ;
/// - la fenetre est introuvable au moment du demarrage.
///
/// L'echec n'est jamais fatal : le mode kwin-eis reste operationnel, mais les
/// clics seront decales si la fenetre n'est pas positionnee en (0, 0).
async fn resolve_kwin_origin(title: Option<&str>, backend: ScreencapBackend) -> (i32, i32) {
    let Some(title) = title else {
        return (0, 0);
    };
    let use_kwin = match backend {
        ScreencapBackend::Kwin => true,
        ScreencapBackend::Xcap => false,
        ScreencapBackend::Auto => is_kde_wayland(),
    };
    if !use_kwin {
        return (0, 0);
    }
    match KwinRenderer::new(title).window_origin().await {
        Ok(origin) => {
            eprintln!(
                "deveye-server : origine fenetre '{}' resolue : ({}, {})",
                title, origin.0, origin.1
            );
            origin
        }
        Err(e) => {
            eprintln!(
                "deveye-server : resolution origine '{}' echouee ({e}), repli sur (0,0)",
                title
            );
            (0, 0)
        }
    }
}

/// Assemble et renvoie le serveur MCP a partir des arguments CLI.
///
/// Toute la logique de composition est ici : selection des adapters selon
/// [`TargetMode`], application des [`Fallback*`][crate::fallback] et
/// `Composite*` (adapter-composite), branchement des ports SDK (state, logs,
/// commandes).
pub(crate) async fn build_mcp(args: &Args) -> DeveyeMcpServer {
    let cdp_filter = build_cdp_filter(args);

    // Pieces du pont natif (SDK) : evaluateur JS via la pompe de l'app,
    // interaction en evenements JS, DOM par outerHTML, yeux par xcap.
    let sdk_eval: Arc<dyn JsEvaluator> = Arc::new(SdkEvaluator::new(args.sdk_url.clone()));
    // Titre de fenetre pour les yeux OS-level ; `None` -> repli plein ecran
    // (MonitorRenderer) au lieu de l'ancien defaut code en dur.
    let screencap_title = args.screencap_window.clone();

    // CDP : connexion PARESSEUSE — un navigateur absent au demarrage (ou
    // ferme en cours de session) n'est pas fatal, chaque appel d'outil
    // (re)tente l'attache. L'essai immediat ne sert qu'a loguer.
    let initial: Option<Arc<CdpClient>> =
        try_initial_cdp(&args.cdp_url, cdp_filter.clone(), args.target).await;

    let (renderer, interaction, dom_inspector, evaluator, window_lister): Pilotage = match args
        .target
    {
        TargetMode::Cdp => {
            let lazy = Arc::new(LazyCdp::new(
                args.cdp_url.clone(),
                cdp_filter.clone(),
                initial,
            ));
            (lazy.clone(), lazy.clone(), lazy.clone(), lazy.clone(), lazy)
        }
        TargetMode::Sdk => (
            build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
            Arc::new(JsInteraction::new(sdk_eval.clone())),
            Arc::new(EvalDomInspector::new(sdk_eval.clone())),
            sdk_eval.clone(),
            Arc::new(EmptyWindowLister),
        ),
        // Auto : CDP en premier ; toute erreur de LIAISON (PortError::Network
        // — navigateur absent/ferme) bascule l'appel sur le pont natif.
        TargetMode::Auto => {
            let lazy = Arc::new(LazyCdp::new(
                args.cdp_url.clone(),
                cdp_filter.clone(),
                initial,
            ));
            (
                Arc::new(FallbackRenderer {
                    primary: lazy.clone(),
                    fallback: build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
                }),
                Arc::new(FallbackInteraction {
                    primary: lazy.clone(),
                    fallback: Arc::new(JsInteraction::new(sdk_eval.clone())),
                }),
                Arc::new(FallbackDom {
                    primary: lazy.clone(),
                    fallback: Arc::new(EvalDomInspector::new(sdk_eval.clone())),
                }),
                Arc::new(FallbackEvaluator {
                    primary: lazy.clone(),
                    fallback: sdk_eval.clone(),
                }),
                lazy,
            )
        }
        // Mains OS-level EIS ; yeux par screencap (M1) ; DOM/JS indisponibles
        // (rendu natif). Le state/les logs restent servis par le SDK plus bas.
        // L'origine fenetre est resolue au demarrage (best-effort, repli (0,0)).
        TargetMode::KwinEis => {
            let window_origin =
                resolve_kwin_origin(screencap_title.as_deref(), args.screencap_backend).await;
            (
                build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
                Arc::new(KwinEisInteraction::new(window_origin).expect("worker EIS")),
                Arc::new(UnsupportedInspector),
                Arc::new(UnsupportedEvaluator),
                Arc::new(EmptyWindowLister),
            )
        }
    };

    // State inspector + logs + commandes logiques via le SDK opt-in cote app
    // cible (toujours servis par le SDK, quelle que soit la cible des yeux).
    let state_inspector: Arc<dyn AppInspector> =
        Arc::new(DioxusSignalsInspector::new(args.sdk_url.clone()));
    let log_source: Arc<dyn LogSource> = Arc::new(SdkTracingLogSource::new(args.sdk_url.clone()));
    let command_router: Arc<dyn CommandRouter> =
        Arc::new(SdkCommandRouter::new(args.sdk_url.clone()));

    let inspector: Arc<dyn AppInspector> = Arc::new(CompositeInspector {
        dom: dom_inspector,
        state: state_inspector,
    });

    // Ports d'orchestration (M6), composes sur les ports deja construits :
    // erreurs console par injection JS, attente conditionnelle multi-predicats.
    //
    // En mode kwin-eis il n'y a pas de WebView : l'evaluateur renverrait
    // PortError::Unsupported a chaque drain. On branche un noop plutot que de
    // propager une fausse erreur (absence de console != erreur du port).
    let console_errors: Arc<dyn ConsoleErrorSource> = if matches!(args.target, TargetMode::KwinEis)
    {
        Arc::new(NoopConsoleErrorSource)
    } else {
        Arc::new(JsConsoleErrorSource {
            evaluator: evaluator.clone(),
        })
    };
    let wait_condition: Arc<dyn WaitCondition> = Arc::new(CompositeWaitCondition {
        evaluator: evaluator.clone(),
        inspector: inspector.clone(),
        renderer: renderer.clone(),
    });

    DeveyeMcpServer::new(DeveyePorts {
        renderer,
        interaction,
        inspector,
        log_source,
        evaluator,
        command_router,
        window_lister,
        console_errors,
        wait_condition,
    })
}
