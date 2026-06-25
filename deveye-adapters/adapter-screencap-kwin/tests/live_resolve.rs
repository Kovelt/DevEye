//! Test d'intégration **live** (session KDE/Wayland requise), `#[ignore]` par
//! défaut. Vérifie que la résolution titre -> UUID aboutit bout en bout : le
//! script KWin éphémère, l'interface `Bridge` servie par zbus, et le callback
//! `callDBus`. La capture elle-même (`CaptureWindow`) exige
//! `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` + relogin ; ce test reste donc
//! valide même sans cette autorisation : il distingue un échec de *résolution*
//! (Unsupported/WindowNotFound) d'un blocage à la *capture* (Render/NoAuthorized).
//!
//! Lancer : `cargo test -p adapter-screencap-kwin --test live_resolve -- --ignored`

use adapter_screencap_kwin::KwinRenderer;
use deveye_ports::{PortError, Renderer};

#[tokio::test]
#[ignore = "requiert une session KDE Plasma Wayland live"]
async fn live_resolve_reaches_capture() {
    // Sous-chaîne vide => matche la première fenêtre listée : on isole ainsi le
    // chemin de résolution, indépendamment des fenêtres ouvertes.
    let renderer = KwinRenderer::new("");
    match renderer.screenshot().await {
        // Auth activée : capture réelle.
        Ok(shot) => {
            assert!(!shot.bytes.is_empty(), "PNG vide");
            eprintln!("capture OK : {} octets", shot.bytes.len());
        }
        // Résolution échouée : aucune fenêtre / script ou callback KO -> bug.
        Err(PortError::Unsupported(m)) => {
            panic!("resolution titre->UUID echouee (capture jamais atteinte) : {m}");
        }
        // Capture atteinte mais refusée (NoAuthorized) : résolution OK, auth à activer.
        Err(e) => {
            let s = e.to_string();
            assert!(
                s.contains("CaptureWindow"),
                "erreur inattendue (resolution OK ?) : {s}"
            );
            eprintln!("resolution OK ; capture gatee par l'auth : {s}");
        }
    }
}
