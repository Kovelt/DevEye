//! Test d'injection EIS **réel** (intrusif) : exige une session KDE Plasma
//! Wayland active (KWin EIS). Ignoré par défaut — il déplace le vrai pointeur.
//!
//! Lancer explicitement :
//! `cargo test -p adapter-interaction-kwin-eis --test live_inject -- --ignored --nocapture`

use adapter_interaction_kwin_eis::KwinEisInteraction;
use deveye_core::{InteractionCommand, Position};
use deveye_ports::InteractionSink;

#[tokio::test]
#[ignore = "intrusif : bouge le vrai pointeur ; exige une session KDE Wayland (KWin EIS)"]
async fn hover_moves_real_pointer() {
    let eis = KwinEisInteraction::new((0, 0)).expect("worker EIS");
    // Un simple déplacement absolu : prouve handshake + seat + device + motion.
    let result = eis
        .dispatch(InteractionCommand::hover(Position::new(400, 400)))
        .await;
    assert!(result.is_ok(), "hover EIS a échoué : {result:?}");
    eprintln!("hover EIS OK — le pointeur a été déplacé en (400, 400)");
}
