//! Tests de sécurité du serveur local du SDK : garde Host, CORS deny-all,
//! saturation de la file d'évaluation. Ils n'ont de sens qu'en debug (en
//! release `init` n'ouvre aucun port : cf. `release_noop.rs`).
//!
//! Les tests de ce fichier partagent un serveur (registre global du SDK) sur un
//! port dédié, distinct de celui de `wire_protocol.rs`.

#![cfg(debug_assertions)]

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::{HOST, ORIGIN};
use reqwest::StatusCode;

const PORT: u16 = 31598;

fn url(path: &str) -> String {
    format!("http://127.0.0.1:{PORT}{path}")
}

/// Démarre le serveur une seule fois pour tous les tests du fichier.
async fn server() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        deveye_sdk::init(deveye_sdk::Config::with_port(PORT));
    }
    // Idempotent : laisse le temps au runtime dédié de lier le port.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", PORT))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn version(resp: &reqwest::Response) -> Option<&str> {
    resp.headers()
        .get("x-deveye-protocol-version")
        .and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn host_guard_rejects_foreign_and_missing_port_hosts() {
    server().await;
    let http = reqwest::Client::new();
    for host in [
        "evil.com",
        "evil.com:31337",
        "127.0.0.1.evil.com",
        "0.0.0.0",
    ] {
        let resp = http
            .get(url("/devtools/state"))
            .header(HOST, host)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "Host {host}");
        assert_eq!(version(&resp), Some("1.0"), "header de version sur le 403");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"], serde_json::json!("Host non local"));
    }
}

#[tokio::test]
async fn host_guard_accepts_loopback_spellings() {
    server().await;
    let http = reqwest::Client::new();
    for host in [
        "localhost",
        "localhost:31598",
        "LOCALHOST",
        "127.0.0.1",
        "127.0.0.1:31598",
        "[::1]",
        "[::1]:31598",
    ] {
        let resp = http
            .get(url("/devtools/state"))
            .header(HOST, host)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "Host {host}");
    }
}

#[tokio::test]
async fn cors_is_deny_all() {
    server().await;
    let http = reqwest::Client::new();

    // Requête simple avec Origin : aucune autorisation cross-origin accordée.
    let resp = http
        .get(url("/devtools/state"))
        .header(HOST, "localhost")
        .header(ORIGIN, "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "aucune origine ne doit etre autorisee"
    );

    // Préflight OPTIONS : pas d'autorisation de méthode ni d'origine.
    let resp = http
        .request(reqwest::Method::OPTIONS, url("/devtools/command"))
        .header(HOST, "localhost")
        .header(ORIGIN, "https://evil.example")
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    assert!(resp.headers().get("access-control-allow-origin").is_none());
    assert!(resp.headers().get("access-control-allow-methods").is_none());
    assert_eq!(version(&resp), Some("1.0"));
}

#[tokio::test]
async fn eval_queue_saturation_answers_503() {
    server().await;
    let mut pump = deveye_sdk::take_eval_requests();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let http = reqwest::Client::new();
    // Remplit la file (capacité 32) : la pompe ne draine pas, ces requêtes
    // restent en attente de réponse.
    let mut pending = Vec::new();
    for i in 0..deveye_sdk::EVAL_QUEUE_CAPACITY {
        let http = http.clone();
        pending.push(tokio::spawn(async move {
            http.post(url("/devtools/eval"))
                .header(HOST, "localhost")
                .json(&serde_json::json!({ "expression": format!("{i}") }))
                .send()
                .await
                .map(|r| r.status())
        }));
    }
    // Attendre que les 32 soient en file (le serveur est asynchrone).
    tokio::time::sleep(Duration::from_millis(500)).await;

    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .json(&serde_json::json!({ "expression": "overflow" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("file d'évaluation pleine"),
        "message de backpressure attendu, recu {body}"
    );

    // Libère les requêtes en attente pour ne pas attendre le timeout de 10 s.
    for _ in 0..deveye_sdk::EVAL_QUEUE_CAPACITY {
        let req = tokio::time::timeout(Duration::from_secs(2), pump.recv())
            .await
            .expect("requete en file")
            .expect("canal ouvert");
        let _ = req.respond.send(Ok(serde_json::json!(null)));
    }
    for handle in pending {
        assert_eq!(handle.await.unwrap().unwrap(), StatusCode::OK);
    }
}
