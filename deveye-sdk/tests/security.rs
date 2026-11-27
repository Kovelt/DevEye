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
        "aucune origine ne doit être autorisee"
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

/// Un `fetch` `no-cors` inter-origine ne peut pas lire la réponse, mais ne doit
/// pas non plus pouvoir vider le tampon de logs (effet de bord du GET).
#[tokio::test]
async fn logs_poll_refuses_cross_site_requests_without_draining() {
    server().await;
    let http = reqwest::Client::new();
    deveye_sdk::install_tracing_layer();
    tracing::info!("garde-poll-inter-origine");

    for (name, value) in [
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-site", "same-site"),
        ("origin", "https://evil.example"),
    ] {
        let resp = http
            .get(url("/devtools/logs/poll"))
            .header(HOST, "localhost")
            .header(name, value)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{name}: {value}");
        assert_eq!(version(&resp), Some("1.0"));
    }

    // Le log est acheminé de façon asynchrone : on réessaie (sans délai fixe)
    // jusqu'à le voir. Les refus ci-dessus n'ont rien drainé.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let resp = http
            .get(url("/devtools/logs/poll"))
            .header(HOST, "localhost")
            .header("sec-fetch-site", "none")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = resp.json().await.unwrap();
        let seen = body["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|e| e["message"] == "garde-poll-inter-origine")
        });
        if seen {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "le log émis avant les refus aurait dû rester dans le tampon"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn eval_rejects_oversized_and_untyped_bodies() {
    server().await;
    let http = reqwest::Client::new();
    let oversized = vec![b'x'; deveye_sdk::MAX_REQUEST_BODY_BYTES + 1];
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .header("content-type", "application/json")
        .body(oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(version(&resp), Some("1.0"));

    // Sans content-type JSON, une page tierce ne peut pas émettre une requête
    // « simple » (text/plain) sans préflight : le serveur la refuse.
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .header("content-type", "text/plain")
        .body(r#"{"expression":"1"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .body(r#"{"expression":"1"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn eval_queue_saturation_answers_503() {
    server().await;
    let mut pump = deveye_sdk::take_eval_requests();
    let http = reqwest::Client::new();

    // Sans délai fixe : on envoie des évaluations jusqu'à obtenir le 503 de
    // backpressure. Une requête acceptée reste en attente (la pompe ne draine
    // pas) ; une requête qui ne répond pas dans le court délai de sondage est
    // donc comptée comme en file. Le 503 « aucune pompe » possible tant que
    // l'enregistrement de la pompe n'a pas abouti est ignoré.
    let mut queued = 0usize;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let overflow_body = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "la file n'a jamais saturé ({queued} requêtes en file)"
        );
        let request = http
            .post(url("/devtools/eval"))
            .header(HOST, "localhost")
            .json(&serde_json::json!({ "expression": queued.to_string() }))
            .send();
        let handle = tokio::spawn(request);
        match tokio::time::timeout(Duration::from_millis(100), async {
            // `JoinHandle` n'est pas `Unpin` après timeout : on le consulte par
            // référence mutable pour ne pas l'abandonner.
            let mut handle = handle;
            let out = (&mut handle).await;
            (out, handle)
        })
        .await
        {
            Ok((Ok(Ok(resp)), _)) if resp.status() == StatusCode::SERVICE_UNAVAILABLE => {
                let body: serde_json::Value = resp.json().await.unwrap();
                if body["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("file d'évaluation pleine")
                {
                    break body;
                }
            }
            Ok(_) => {}
            Err(_) => queued += 1,
        }
    };
    assert!(
        queued >= deveye_sdk::EVAL_QUEUE_CAPACITY,
        "saturation avant capacité : {queued} requêtes en file, {overflow_body}"
    );

    // Libère les requêtes en attente pour ne pas attendre le timeout de 10 s.
    for _ in 0..queued {
        let req = tokio::time::timeout(Duration::from_secs(2), pump.recv())
            .await
            .expect("requête en file")
            .expect("canal ouvert");
        let _ = req.respond.send(Ok(serde_json::json!(null)));
    }
}
