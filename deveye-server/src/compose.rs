//! Cablage de composition : selectionne et assemble les adapters selon
//! `TargetMode` et le backend OS-level choisi.
//!
//! Point d'entree principal : [`build_mcp`].

use std::sync::Arc;

use anyhow::Context as _;
use tracing::{info, warn};

use adapter_cdp_webview::CdpClient;
use adapter_clipboard_wayland::WaylandClipboard;
use adapter_commands_http::SdkCommandRouter;
use adapter_composite::{
    CompositeInspector, CompositeWaitCondition, JsConsoleErrorSource, MaskingCommandRouter,
    NoopConsoleErrorSource, RouteDomInspector, RouteWidgetsElements, DEFAULT_DOM_ROUTE,
};
use adapter_dioxus_signals::{
    DioxusSignalsInspector, EvalDomInspector, JsInteraction, SdkEvaluator,
};
use adapter_interaction_kwin_eis::KwinEisInteraction;
use adapter_logs_tracing::SdkTracingLogSource;
use adapter_mcp_server::{DeveyeMcpServer, DeveyePorts, EffectiveBackendProbe, TargetInfo};
use adapter_screencap_kwin::{
    KwinRenderer, KwinWindowGeometry, KwinWindowLister, KwinWindowManager,
};
use adapter_screencap_xcap::{MonitorRenderer, XcapRenderer};
use adapter_x11::{X11Config, X11Target};
use deveye_ports::{
    AppInspector, Clipboard, CommandRouter, ConsoleErrorSource, ElementInspector, InteractionSink,
    JsEvaluator, LogSource, Renderer, WaitCondition, WindowGeometry, WindowLister, WindowManager,
};

use crate::cdp::{build_cdp_filter, try_initial_cdp, LazyCdp};
use crate::cli::{Args, ScreencapBackend, TargetMode};
use crate::fallback::{
    FallbackDom, FallbackEvaluator, FallbackInteraction, FallbackRenderer, FallbackState,
};
use crate::stubs::{
    EmptyWindowLister, UnsupportedClipboard, UnsupportedElementInspector, UnsupportedEvaluator,
    UnsupportedInspector, UnsupportedWindowGeometry, UnsupportedWindowManager,
};

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

/// Backend de capture OS-level effectif : KWin (ScreenShot2, KWin Scripting)
/// ou non. Seul point de décision, partagé par les yeux, la géométrie, le
/// focus, le lister de fenêtres et `target_info` (G45) : `--screencap-backend
/// kwin|xcap` force, `auto` suit la session.
fn use_kwin(backend: ScreencapBackend) -> bool {
    kwin_selected(backend, is_kde_wayland())
}

/// Cœur pur de [`use_kwin`] : la session KDE/Wayland est passée en paramètre.
fn kwin_selected(backend: ScreencapBackend, kde_wayland: bool) -> bool {
    match backend {
        ScreencapBackend::Kwin => true,
        ScreencapBackend::Xcap => false,
        ScreencapBackend::Auto => kde_wayland,
    }
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
        info!("yeux = MonitorRenderer (plein écran ; --screencap-window absent)");
        return Arc::new(MonitorRenderer);
    };
    let use_kwin = use_kwin(backend);
    if use_kwin {
        info!("yeux = KWin D-Bus ScreenShot2 (Wayland natif KDE)");
        Arc::new(KwinRenderer::new(title))
    } else {
        info!("yeux = xcap (X11 / XWayland / Wayland non-KDE)");
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
    let use_kwin = use_kwin(backend);
    if !use_kwin {
        return (0, 0);
    }
    match KwinRenderer::new(title).window_origin().await {
        Ok(origin) => {
            info!(
                titre = title,
                x = origin.0,
                y = origin.1,
                "origine de la fenêtre résolue"
            );
            origin
        }
        Err(e) => {
            warn!(
                titre = title,
                erreur = %e,
                "résolution de l'origine de la fenêtre échouée, repli sur (0, 0)"
            );
            (0, 0)
        }
    }
}

/// Vrai si la session est Wayland (`wl-copy`/`wl-paste` exploitables).
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Construit le presse-papiers (outils `clipboard_get`/`clipboard_set`).
/// `WaylandClipboard` sous Wayland, stub `Unsupported` sinon.
fn build_clipboard() -> Arc<dyn Clipboard> {
    if is_wayland() {
        Arc::new(WaylandClipboard::new())
    } else {
        Arc::new(UnsupportedClipboard)
    }
}

/// Construit le gestionnaire de fenetres (outil `focus_window`). Disponible
/// uniquement quand le screencap passe par KWin (KDE/Wayland) et qu'un titre de
/// fenetre est fourni ; sinon stub `Unsupported`.
fn build_window_manager(title: Option<&str>, backend: ScreencapBackend) -> Arc<dyn WindowManager> {
    let use_kwin = use_kwin(backend);
    match (use_kwin, title) {
        (true, Some(t)) => Arc::new(KwinWindowManager::new(t)),
        _ => Arc::new(UnsupportedWindowManager),
    }
}

/// Construit le port de geometrie de fenetre (outil `window_geometry`). Actif
/// des que le screencap passe par KWin, meme SANS `--screencap-window` : le
/// titre eventuel sert seulement de defaut quand l'appel MCP n'en fournit pas
/// (`KwinWindowGeometry::geometry` rejette proprement un titre vide). Sinon
/// stub `Unsupported`. (revue T4 : ne pas exiger `--screencap-window` alors
/// qu'un titre peut etre passe par l'appel.)
fn build_window_geometry(
    title: Option<&str>,
    backend: ScreencapBackend,
) -> Arc<dyn WindowGeometry> {
    let use_kwin = use_kwin(backend);
    if use_kwin {
        Arc::new(KwinWindowGeometry::new(title.unwrap_or("")))
    } else {
        Arc::new(UnsupportedWindowGeometry)
    }
}

/// Construit le lister de fenetres pour la cible kwin-eis (outil
/// `list_windows`). `KwinWindowLister` enumere tout le workspace et n'a PAS
/// besoin d'un titre : actif des que le screencap passe par KWin, meme sans
/// `--screencap-window` (revue T4 — c'est justement l'outil qui sert a
/// DECOUVRIR le titre a passer ensuite).
fn build_kwin_window_lister(backend: ScreencapBackend) -> Arc<dyn WindowLister> {
    let use_kwin = use_kwin(backend);
    if use_kwin {
        Arc::new(KwinWindowLister::new())
    } else {
        Arc::new(EmptyWindowLister)
    }
}

/// Ce que la session hôte permet, découvert une fois au démarrage : fixe les
/// capacités annoncées par `target_info` (G43). Évite d'annoncer `list_windows`
/// ou `focus_window` sur une session où KWin ne répondra jamais.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostCaps {
    /// Les yeux OS-level passent par KWin (donc KWin Scripting : fenêtres,
    /// géométrie, focus).
    kwin: bool,
    /// Session Wayland (`wl-copy`/`wl-paste` exploitables).
    wayland: bool,
    /// `--screencap-window` fourni : `focus_window` en dépend (le gestionnaire
    /// KWin est construit avec ce titre).
    window_title: bool,
}

impl HostCaps {
    fn detect(args: &Args) -> Self {
        Self {
            kwin: use_kwin(args.screencap_backend()),
            wayland: is_wayland(),
            window_title: args.screencap_window.is_some(),
        }
    }
}

/// Outils qui exigent un moteur JS (webview CDP ou pompe SDK) ; ils répondent
/// `Unsupported` quand `eval_js` est faux. `wait_condition` n'en dépend que
/// pour ses prédicats `dom-selector` et `text-content`.
const JS_DEPENDENT_TOOLS: &[&str] = &[
    "eval_js",
    "click_selector",
    "click_text",
    "find_element",
    "assert_selector",
    "screenshot_element",
    "wait_for_selector",
    "wait_for_text",
    "collect_console_errors",
];

/// Les cibles dont les éléments viennent de la route `ui.widgets` de l'app : les
/// natives, sans DOM (`snapshot` et `act`, expérimentaux, ADR-0073).
fn has_widget_elements(target: TargetMode) -> bool {
    matches!(target, TargetMode::X11 | TargetMode::KwinEis)
}

/// Marche à suivre sur une cible à webview, où les références d'éléments ne sont
/// pas encore câblées (pont WebKitGTK puis CDP, feuille de route 1.2).
const WEBVIEW_ELEMENTS_HINT: &str =
    "les références d'éléments ne sont pas encore disponibles sur les cibles à webview (pont \
     WebKitGTK puis CDP à venir) ; en attendant, `dom`, `find_element`, `click_selector` et \
     `click_text` désignent par sélecteur ou par texte";

/// Éléments par référence de la cible : widgets natifs par la route `ui.widgets`
/// (x11, kwin-eis), indisponibles ailleurs.
fn build_elements(args: &Args) -> Arc<dyn ElementInspector> {
    if has_widget_elements(args.target) {
        let router: Arc<dyn CommandRouter> = Arc::new(SdkCommandRouter::new(args.sdk_url.clone()));
        Arc::new(RouteWidgetsElements::new(router, DEFAULT_DOM_ROUTE))
    } else {
        Arc::new(UnsupportedElementInspector::new(WEBVIEW_ELEMENTS_HINT))
    }
}

/// Construit la description de la cible (outil `target_info`) selon le mode
/// choisi et la session : capacités distinctives, version du serveur, notes sur
/// les pièges et indisponibilités propres à chaque cible.
fn build_target_info(target: TargetMode, host: HostCaps) -> TargetInfo {
    // Fenêtres, géométrie et focus par KWin (hors x11, qui a les siens).
    let kwin_windows = host.kwin;
    let kwin_focus = host.kwin && host.window_title;
    let wm_kwin = if host.kwin { "kwin" } else { "aucun" };
    let clipboard = host.wayland;
    let mut info = TargetInfo {
        target: target.name().to_string(),
        effective_backend: Some(target.name().to_string()),
        eyes: String::new(),
        hands: String::new(),
        eval_js: target.has_js(),
        dom: true,
        state: true,
        console_errors: target.has_js(),
        list_windows: false,
        window_geometry: kwin_windows,
        focus_window: kwin_focus,
        window_manager: wm_kwin.to_string(),
        clipboard,
        elements: has_widget_elements(target),
        js_dependent_tools: JS_DEPENDENT_TOOLS.iter().map(|t| t.to_string()).collect(),
        server: crate::version::server_version(),
        notes: String::new(),
    };
    match target {
        TargetMode::Auto => {
            info.effective_backend = Some("indetermine".to_string());
            info.eyes = "CDP Page.captureScreenshot, repli screencap OS (KWin/xcap)".to_string();
            info.hands =
                "CDP Input.* (events natifs), repli events JS synthetiques (pont SDK)".to_string();
            info.list_windows = true;
            info.notes = "Repli appel-par-appel sur le pont SDK des que le CDP est injoignable ; \
                    `effective_backend` dit lequel sert (cdp, sdk, indetermine) et la première \
                    bascule est journalisée. list_windows reste CDP : il échoue quand le CDP \
                    est tombe. Les capacités webview (eval_js/dom) supposent un webview joignable \
                    (CDP ou pompe SDK)."
                .to_string();
        }
        TargetMode::Cdp => {
            info.eyes = "CDP Page.captureScreenshot".to_string();
            info.hands = "CDP Input.* (events natifs navigateur, isTrusted=true)".to_string();
            info.list_windows = true;
            info.notes = "state/logs/commandes dependent du SDK opt-in embarque dans l'app — \
                    absent en cible WASM/.NET pure (renvoie alors une erreur réseau). \
                    window_geometry/focus_window passent par KWin (KDE/Wayland) et non par le CDP."
                .to_string();
        }
        TargetMode::Sdk => {
            info.eyes = "screencap OS (KWin sous Wayland/KDE, xcap sinon)".to_string();
            info.hands = "events JS synthétiques via la pompe document::eval de l'app".to_string();
            info.notes = "resize indisponible (emulation des métriques moteur = CDP). Pas de \
                    visibilite inter-fenetres (list_windows vide). window_geometry/focus_window \
                    necessitent KDE/Wayland."
                .to_string();
        }
        TargetMode::KwinEis => {
            info.eyes = "screencap OS (KWin sous Wayland/KDE, xcap sinon)".to_string();
            info.hands = "injection OS-level via KWin EIS (souris/clavier reels)".to_string();
            info.dom = false;
            info.list_windows = kwin_windows;
            info.notes =
                "Rendu natif, pas de WebView : dom/eval_js/find_element \
                    renvoient Unsupported, navigate indisponible ; wait_for_selector/wait_for_text \
                    attendent le timeout complet puis répondent found:false. state/logs via le SDK/shim \
                    de l'app si lance. Yeux et assertions visuelles : screenshot/assert_visual. \
                    list_windows/window_geometry/focus_window via KWin Scripting : necessitent \
                    KDE/Wayland (ces champs sont faux ailleurs). list_windows enumere tout le \
                    workspace (aucun titre requis) ; window_geometry cible par titre (celui de \
                    l'appel ou, a défaut, --screencap-window) ; focus_window exige \
                    --screencap-window au démarrage. snapshot/act (expérimentaux) désignent \
                    les widgets de la route `ui.widgets` par des références `w:<id>`."
                    .to_string();
        }
        TargetMode::X11 => {
            info.eyes = "GetImage du serveur X privé, rogne a la fenêtre (--window)".to_string();
            info.hands =
                "injection XTEST réelle sur le serveur X privé (souris, molette, clavier du serveur)"
                    .to_string();
            info.list_windows = true;
            info.window_geometry = true;
            info.focus_window = true;
            info.window_manager = "x11".to_string();
            info.clipboard = false;
            info.notes =
                "Serveur X explicite (--display), jamais la session vivante : display < 100, \
                    $DISPLAY et XWayland refuses. Pas de WebView : eval_js/find_element/\
                    click_text renvoient Unsupported, navigate/handle_dialog/\
                    upload_file aussi ; wait_for_selector/wait_for_text attendent le timeout \
                    complet puis répondent found:false. dom = réponse de la route `ui.widgets` de l'app (rects en \
                    coordonnees fenetre : cliquer au centre avec `click`), vide d'erreur Inspect \
                    si l'app ne l'expose pas ; state/logs/commandes via le SDK (--sdk-url). \
                    Attentes : wait_condition (state-key, screenshot-stable, screenshot-region-stable, \
                    log-message, target-alive ; pas dom-selector ni text-content). \
                    Clavier = celui du serveur X ; un caractere absent est injecte par remappage \
                    temporaire. Molette : un cran par tranche de 120 px de dx/dy. Presse-papiers \
                    indisponible. Limites de capture : l'image est la racine du serveur X \
                    rognee a la fenêtre (une fenêtre recouvrante y apparait, la partie hors \
                    écran est noire), sans mise a l'échelle HiDPI ni compositeur. \
                    snapshot/act (expérimentaux) désignent les widgets de `ui.widgets` par \
                    des références `w:<id>`, position remesurée à chaque action."
                    .to_string();
        }
    }
    info
}

/// Assemble et renvoie le serveur MCP a partir des arguments CLI.
///
/// Toute la logique de composition est ici : selection des adapters selon
/// [`TargetMode`], application des [`Fallback*`][crate::fallback] et
/// `Composite*` (adapter-composite), branchement des ports SDK (state, logs,
/// commandes).
///
/// Retourne `Err` si un adapter refuse de demarrer (ex : thread worker EIS,
/// cf. `TargetMode::KwinEis` ci-dessous) — propage jusqu'a `main()` plutot que
/// de paniquer (seul point de panique hors tests du binaire avant ce fix, A6).
pub(crate) async fn build_mcp(args: &Args) -> anyhow::Result<DeveyeMcpServer> {
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
        try_initial_cdp(args.cdp_url(), cdp_filter.clone(), args.target).await;

    // Mode `auto` : état du repli, lu par `target_info` (ADR-0034).
    let cdp_up = initial.is_some();
    let mut fallback_state: Option<Arc<FallbackState>> = None;

    // Cible X11 (si choisie) : fournit aussi le focus et la geometrie de fenetre,
    // sans passer par les helpers KWin (leur predicat de session ne s'applique pas).
    let mut x11_target: Option<Arc<X11Target>> = None;

    let (renderer, interaction, dom_inspector, evaluator, window_lister): Pilotage =
        match args.target {
            TargetMode::Cdp => {
                let lazy = Arc::new(LazyCdp::new(
                    args.cdp_url().to_string(),
                    cdp_filter.clone(),
                    initial,
                ));
                (lazy.clone(), lazy.clone(), lazy.clone(), lazy.clone(), lazy)
            }
            TargetMode::Sdk => (
                build_os_renderer(screencap_title.as_deref(), args.screencap_backend()),
                Arc::new(JsInteraction::new(sdk_eval.clone())),
                Arc::new(EvalDomInspector::new(sdk_eval.clone())),
                sdk_eval.clone(),
                Arc::new(EmptyWindowLister),
            ),
            // Auto : CDP en premier ; toute erreur de LIAISON (PortError::Network
            // — navigateur absent/ferme) bascule l'appel sur le pont natif.
            TargetMode::Auto => {
                let lazy = Arc::new(LazyCdp::new(
                    args.cdp_url().to_string(),
                    cdp_filter.clone(),
                    initial,
                ));
                let state = Arc::new(FallbackState::new(cdp_up));
                fallback_state = Some(state.clone());
                (
                    Arc::new(FallbackRenderer {
                        primary: lazy.clone(),
                        fallback: build_os_renderer(
                            screencap_title.as_deref(),
                            args.screencap_backend(),
                        ),
                        state: state.clone(),
                    }),
                    Arc::new(FallbackInteraction {
                        primary: lazy.clone(),
                        fallback: Arc::new(JsInteraction::new(sdk_eval.clone())),
                        state: state.clone(),
                    }),
                    Arc::new(FallbackDom {
                        primary: lazy.clone(),
                        fallback: Arc::new(EvalDomInspector::new(sdk_eval.clone())),
                        state: state.clone(),
                    }),
                    Arc::new(FallbackEvaluator {
                        primary: lazy.clone(),
                        fallback: sdk_eval.clone(),
                        state,
                    }),
                    lazy,
                )
            }
            // Mains OS-level EIS ; yeux par screencap (M1) ; DOM/JS indisponibles
            // (rendu natif). Le state/les logs restent servis par le SDK plus bas.
            // L'origine fenêtre est résolue au démarrage (best-effort, repli (0,0)).
            TargetMode::KwinEis => {
                let window_origin =
                    resolve_kwin_origin(screencap_title.as_deref(), args.screencap_backend()).await;
                // KwinEisInteraction::new échoue seulement si l'OS refuse la création
                // du thread worker (RLIMIT_NPROC, OOM...) : propage au lieu du
                // `.expect` d'origine, qui paniquait tout le binaire (A6).
                let interaction = KwinEisInteraction::new(window_origin)
                    .context("--target kwin-eis : démarrage du thread worker EIS impossible")?;
                (
                    build_os_renderer(screencap_title.as_deref(), args.screencap_backend()),
                    Arc::new(interaction),
                    Arc::new(UnsupportedInspector),
                    Arc::new(UnsupportedEvaluator),
                    build_kwin_window_lister(args.screencap_backend()),
                )
            }
            // Serveur X prive explicite : yeux GetImage, mains XTEST, fenetres
            // QueryTree. Le DOM est la route `ui.widgets` de l'app (via le SDK) ;
            // pas de JS. Aucun build_kwin_* ici.
            TargetMode::X11 => {
                let display = args.display.clone().context(
                    "--target x11 : --display est obligatoire (aucun repli sur $DISPLAY)",
                )?;
                let target = Arc::new(
                    X11Target::new(X11Config {
                        display,
                        window: screencap_title.clone(),
                        class: args.x11_class.clone(),
                    })
                    .context("--target x11 : display refuse par la garde d'isolation")?,
                );
                info!(
                    display = %target.display_id(),
                    fenetre = screencap_title.as_deref().unwrap_or("écran entier"),
                    "cible x11 (serveur privé)"
                );
                let router: Arc<dyn CommandRouter> =
                    Arc::new(SdkCommandRouter::new(args.sdk_url.clone()));
                x11_target = Some(target.clone());
                (
                    target.clone(),
                    target.clone(),
                    Arc::new(RouteDomInspector::new(router, DEFAULT_DOM_ROUTE)),
                    Arc::new(UnsupportedEvaluator),
                    target,
                )
            }
        };

    // State inspector + logs + commandes logiques via le SDK opt-in cote app
    // cible (toujours servis par le SDK, quelle que soit la cible des yeux).
    let state_inspector: Arc<dyn AppInspector> =
        Arc::new(DioxusSignalsInspector::new(args.sdk_url.clone()));
    let log_source: Arc<dyn LogSource> = Arc::new(SdkTracingLogSource::new(args.sdk_url.clone()));
    let command_router: Arc<dyn CommandRouter> = Arc::new(MaskingCommandRouter::new(
        Arc::new(SdkCommandRouter::new(args.sdk_url.clone())),
        DEFAULT_DOM_ROUTE,
    ));

    let inspector: Arc<dyn AppInspector> = Arc::new(CompositeInspector {
        dom: dom_inspector,
        state: state_inspector,
    });

    // Ports d'orchestration (M6), composes sur les ports deja construits :
    // erreurs console par injection JS, attente conditionnelle multi-predicats.
    //
    // Sur une cible native (kwin-eis, x11) il n'y a pas de WebView : l'evaluateur renverrait
    // PortError::Unsupported a chaque drain. On branche un noop plutot que de
    // propager une fausse erreur (absence de console != erreur du port).
    let console_errors: Arc<dyn ConsoleErrorSource> = if args.target.has_js() {
        Arc::new(JsConsoleErrorSource {
            evaluator: evaluator.clone(),
        })
    } else {
        Arc::new(NoopConsoleErrorSource)
    };
    let wait_condition: Arc<dyn WaitCondition> = Arc::new(CompositeWaitCondition {
        evaluator: evaluator.clone(),
        inspector: inspector.clone(),
        renderer: renderer.clone(),
        log_source: log_source.clone(),
    });

    // Focus et geometrie : l'adapter X11 les sert lui-meme ; sinon KWin/stubs.
    let (window_manager, window_geometry): (Arc<dyn WindowManager>, Arc<dyn WindowGeometry>) =
        match &x11_target {
            Some(t) => (t.clone(), t.clone()),
            None => (
                build_window_manager(screencap_title.as_deref(), args.screencap_backend()),
                build_window_geometry(screencap_title.as_deref(), args.screencap_backend()),
            ),
        };
    // Presse-papiers : jamais celui de la session vivante depuis une cible X11
    // isolee (le stub generique repond Unsupported).
    let clipboard: Arc<dyn Clipboard> = if x11_target.is_some() {
        Arc::new(UnsupportedClipboard)
    } else {
        build_clipboard()
    };

    Ok(DeveyeMcpServer::new(DeveyePorts {
        renderer,
        interaction,
        inspector,
        log_source,
        evaluator,
        command_router,
        window_lister,
        console_errors,
        wait_condition,
        window_manager,
        window_geometry,
        clipboard,
        elements: build_elements(args),
        target_info: build_target_info(args.target, HostCaps::detect(args)),
        effective_backend: fallback_state
            .map(|state| -> EffectiveBackendProbe { Arc::new(move || state.label().to_string()) }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(kwin: bool, wayland: bool) -> HostCaps {
        HostCaps {
            kwin,
            wayland,
            window_title: false,
        }
    }

    /// Le choix KWin/xcap : forcé par le flag, sinon dicté par la session.
    #[test]
    fn kwin_selection_follows_flag_then_session() {
        use ScreencapBackend::*;
        for (backend, kde, expected) in [
            (Kwin, false, true),
            (Kwin, true, true),
            (Xcap, true, false),
            (Xcap, false, false),
            (Auto, true, true),
            (Auto, false, false),
        ] {
            assert_eq!(
                kwin_selected(backend, kde),
                expected,
                "{backend:?} kde={kde}"
            );
        }
    }

    /// Hors KDE/Wayland, kwin-eis n'annonce ni fenêtres, ni géométrie, ni focus
    /// (G43) ; sous KDE/Wayland il les annonce, le focus exigeant un titre.
    #[test]
    fn kwin_eis_capabilities_depend_on_the_session() {
        let off = build_target_info(TargetMode::KwinEis, host(false, false));
        assert!(!off.list_windows && !off.window_geometry && !off.focus_window);
        assert_eq!(off.window_manager, "aucun");
        assert!(!off.clipboard);
        let on = build_target_info(TargetMode::KwinEis, host(true, true));
        assert!(on.list_windows && on.window_geometry && !on.focus_window);
        assert_eq!(on.window_manager, "kwin");
        assert!(on.clipboard);
        let titled = build_target_info(
            TargetMode::KwinEis,
            HostCaps {
                window_title: true,
                ..host(true, true)
            },
        );
        assert!(titled.focus_window);
        assert!(!on.eval_js && !on.dom && !on.console_errors);
    }

    /// x11 : presse-papiers jamais, fenêtres par le serveur X quelle que soit la session.
    #[test]
    fn x11_ignores_the_host_session() {
        let info = build_target_info(TargetMode::X11, host(true, true));
        assert!(info.window_geometry && info.focus_window && info.list_windows);
        assert_eq!(info.window_manager, "x11");
        assert!(!info.clipboard);
    }

    /// Le backend effectif initial vaut la cible, sauf en `auto` (indéterminé
    /// tant que le repli n'a pas tranché) ; la version du serveur est exposée.
    #[test]
    fn target_info_carries_backend_and_server_version() {
        let sdk = build_target_info(TargetMode::Sdk, host(false, false));
        assert_eq!(sdk.effective_backend.as_deref(), Some("sdk"));
        let auto = build_target_info(TargetMode::Auto, host(false, false));
        assert_eq!(auto.effective_backend.as_deref(), Some("indetermine"));
        assert_eq!(auto.server.version, env!("CARGO_PKG_VERSION"));
        assert!(auto.js_dependent_tools.iter().any(|t| t == "eval_js"));
        assert!(auto.eval_js && auto.console_errors);
    }

    /// La description de la cible X11 annonce les capacites reelles : dom par
    /// route (true), pas de JS ni de console, state du SDK, fenetres listables.
    #[test]
    fn x11_target_info_matches_the_wiring() {
        let info = build_target_info(TargetMode::X11, host(false, false));
        assert_eq!(info.target, "x11");
        assert!(info.dom && info.state && info.list_windows);
        assert!(!info.eval_js && !info.console_errors);
        assert!(info.eyes.contains("GetImage") && info.hands.contains("XTEST"));
        assert!(info.notes.contains("ui.widgets") && info.notes.contains("--display"));
    }

    /// `snapshot`/`act` : annoncés pour les cibles à arbre de widgets, pas ailleurs.
    #[test]
    fn elements_are_announced_only_for_widget_targets() {
        for (target, expected) in [
            (TargetMode::X11, true),
            (TargetMode::KwinEis, true),
            (TargetMode::Auto, false),
            (TargetMode::Cdp, false),
            (TargetMode::Sdk, false),
        ] {
            let info = build_target_info(target, host(false, false));
            assert_eq!(info.elements, expected, "{target:?}");
            assert_eq!(has_widget_elements(target), expected, "{target:?}");
        }
    }

    /// Sur une cible à webview, `snapshot` répond `unsupported` avec une marche à suivre.
    #[tokio::test]
    async fn webview_targets_answer_unsupported_with_a_hint() {
        use clap::Parser as _;
        use deveye_core::elements::SnapshotQuery;
        let args = Args::try_parse_from(["deveye-server", "--target", "sdk"]).expect("args");
        match build_elements(&args).snapshot(&SnapshotQuery::new()).await {
            Err(deveye_ports::ElementError::Unsupported { hint }) => {
                assert!(hint.contains("click_selector"), "{hint}")
            }
            other => panic!("attendu Unsupported : {:?}", other.map(|s| s.count())),
        }
    }

    /// `has_js` pilote la console : les cibles natives n'ont pas de console JS.
    #[test]
    fn native_targets_have_no_js_console() {
        assert!(!TargetMode::X11.has_js() && !TargetMode::KwinEis.has_js());
        assert!(TargetMode::Cdp.has_js() && TargetMode::Sdk.has_js() && TargetMode::Auto.has_js());
    }

    fn x11_args(extra: &[&str]) -> Args {
        use clap::Parser as _;
        Args::try_parse_from(
            ["deveye-server", "--target", "x11"]
                .into_iter()
                .chain(extra.iter().copied()),
        )
        .expect("arguments valides")
    }

    /// La garde refuse a la composition (donc au demarrage du serveur), avant tout
    /// acces reseau ou X : display bas, forme non locale.
    #[tokio::test]
    async fn x11_composition_refuses_unsafe_displays() {
        for bad in [":0", ":1", ":99", "localhost:250", "250"] {
            let err = build_mcp(&x11_args(&["--display", bad]))
                .await
                .map(|_| ())
                .unwrap_err();
            let msg = format!("{err:#}");
            assert!(msg.contains("garde"), "{bad} : {msg}");
        }
    }

    /// Un display valide compose sans se connecter (connexion paresseuse).
    #[tokio::test]
    async fn x11_composition_is_lazy() {
        // :258 : valide pour la garde ; aucun serveur n'est requis a la composition.
        let ok = build_mcp(&x11_args(&["--display", ":258", "--window", "Synth"])).await;
        // Un autre uid pourrait posseder X258 : la garde refuserait alors, ce qui
        // reste un refus propre (jamais un panic).
        if let Err(e) = ok {
            assert!(format!("{e:#}").contains("garde"), "{e:#}");
        }
    }
}
