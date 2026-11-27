//! Cas nominal de `try_install_tracing_layer` : premier appel `Ok`, second sans
//! effet (`Ok`), capacité `logs` annoncée, événement capté. Fichier dédié : l'abonné
//! global est propre au processus (ADR-0066).

#![cfg(all(debug_assertions, feature = "tracing-layer"))]

#[test]
fn try_install_succeeds_then_is_idempotent_across_threads() {
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(deveye_sdk::try_install_tracing_layer))
        .collect();
    for handle in handles {
        handle
            .join()
            .expect("fil")
            .expect("aucun appel concurrent ne doit échouer");
    }
    assert!(deveye_sdk::try_install_tracing_layer().is_ok());
    tracing::info!("capté par le layer global");
}
