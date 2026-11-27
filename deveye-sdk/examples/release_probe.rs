//! Exemple minimal d'intégration du SDK, volontairement SANS garde
//! `#[cfg(debug_assertions)]` autour des appels : il sert de sonde à la preuve
//! « le SDK est un no-op en release » (`scripts/verify-sdk-release-noop.sh`,
//! recette `just release-noop`, étape de la CI). Le binaire release de cette
//! sonde ne doit contenir ni le serveur HTTP ni ses routes `/devtools/`.

fn main() {
    deveye_sdk::init(deveye_sdk::Config::with_port(31598));
    deveye_sdk::install_tracing_layer();
    deveye_sdk::set_state("probe", serde_json::json!({ "ok": true }));
    println!("sonde terminée");
}
