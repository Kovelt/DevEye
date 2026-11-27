//! `try_install_tracing_layer` signale l'abonné global déjà présent au lieu de
//! l'ignorer (ADR-0066). Fichier dédié : l'abonné global est propre au processus.

#![cfg(debug_assertions)]

#[test]
fn try_install_reports_existing_global_subscriber() {
    tracing::subscriber::set_global_default(tracing_subscriber::registry())
        .expect("premier abonné global");
    let err = deveye_sdk::try_install_tracing_layer().expect_err("abonné déjà installé");
    assert!(matches!(
        err,
        deveye_sdk::TracingInstallError::SubscriberAlreadySet
    ));
    // La variante historique ignore l'échec sans paniquer.
    deveye_sdk::install_tracing_layer();
}
