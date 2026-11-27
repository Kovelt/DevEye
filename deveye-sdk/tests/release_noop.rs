//! Filet de sécurité release : `init()` n'ouvre AUCUN port hors
//! `debug_assertions`. Compilé uniquement en release ; à lancer avec
//! `cargo test --release -p deveye-sdk`.

#![cfg(not(debug_assertions))]

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const PORT: u16 = 31597;

// En release, `ui::is_observed` est constante et fausse : l'app ne construit jamais
// d'arbre, et la vérification a lieu à la compilation.
const _: () = assert!(!deveye_sdk::ui::is_observed());

#[test]
fn init_opens_no_port_in_release() {
    deveye_sdk::init(deveye_sdk::Config::with_port(PORT));
    deveye_sdk::install_tracing_layer();
    deveye_sdk::ui::publish(deveye_sdk::ui::builder(10, 10).build());
    assert!(!deveye_sdk::ui::is_observed());
    // Laisser à un éventuel serveur le temps de démarrer : la connexion doit
    // rester refusée tout du long.
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err(),
            "le SDK a ouvert un port en release"
        );
    }
}
