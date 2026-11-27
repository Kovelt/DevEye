//! Le SDK Rust, démarré en processus sur un port éphémère, doit passer la
//! suite à 100 % : aucun échec et, dans la seconde phase, aucune vérification
//! ignorée. Le SDK a un état global : ce fichier est son propre processus de test.

use std::time::Duration;

use deveye_conformance::{run, Options};

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port éphémère");
    listener.local_addr().expect("adresse locale").port()
}

async fn wait_until_listening(port: u16) {
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("le SDK n'écoute pas sur le port {port}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdk_rust_passes_the_whole_suite() {
    // `init` est un no-op en release : rien à valider alors.
    if !cfg!(debug_assertions) {
        return;
    }
    let port = free_port();
    deveye_sdk::init(deveye_sdk::Config::with_port(port));
    wait_until_listening(port).await;

    deveye_sdk::set_state("password", serde_json::json!("hunter2"));
    deveye_sdk::set_state("page", serde_json::json!("Agenda"));
    for name in ["zeta", "alpha", "ping"] {
        deveye_sdk::expose_route(
            name,
            "commande de test",
            |_| serde_json::json!({ "ok": true }),
        );
    }
    let url = format!("http://127.0.0.1:{port}");

    // Phase 1 : ni pompe d'évaluation ni couche tracing (eval répond 503).
    let report = run(&Options::new(&url)).await.expect("suite exécutable");
    println!("{}", report.render());
    assert!(report.is_conformant(), "écarts en phase 1");

    // Phase 2 : pompe, logs et expressions spéciales : tout est observable.
    deveye_sdk::install_tracing_layer();
    tracing::info!(password = "log-pw", user = "alice", "premier événement");
    tracing::warn!(token = "log-tok", "second événement");
    let mut pump = deveye_sdk::take_eval_requests();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Some(request) = pump.recv().await {
            match request.expression.as_str() {
                // Garder l'émetteur vivant sans répondre : le SDK finit en 504.
                "slow" => held.push(request.respond),
                // Lâcher l'émetteur : le SDK répond 500.
                "abandon" => drop(request.respond),
                _ => {
                    let _ = request.respond.send(Ok(serde_json::json!(2)));
                }
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut options = Options::new(&url);
    options.slow_expression = Some("slow".into());
    options.abandon_expression = Some("abandon".into());
    options.invoke_route = Some("ping".into());
    let report = run(&options).await.expect("suite exécutable");
    println!("{}", report.render());
    assert!(
        report.is_fully_checked(),
        "écarts ou vérifications ignorées en phase 2"
    );
}

#[tokio::test]
async fn unreachable_implementation_is_reported_apart() {
    let port = free_port();
    let error = run(&Options::new(format!("http://127.0.0.1:{port}")))
        .await
        .expect_err("rien n'écoute");
    assert!(matches!(
        error,
        deveye_conformance::ConformanceError::Unreachable { .. }
    ));
}

/// Un serveur qui répond `200 {}` à tout (ni garde Host, ni en-tête de version,
/// ni 404) doit être déclaré non conforme : la suite ne se contente pas de passer.
#[tokio::test]
async fn non_conformant_server_is_detected() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("adresse").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                          content-length: 2\r\nconnection: close\r\n\r\n{}",
                    )
                    .await;
            });
        }
    });
    let report = run(&Options::new(format!("http://127.0.0.1:{port}")))
        .await
        .expect("suite exécutable");
    assert!(!report.is_conformant());
    assert!(report.failed() >= 10, "{}", report.render());
}
