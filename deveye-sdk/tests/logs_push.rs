//! Push de logs sans abonné `tracing` global (ADR-0066) : `log`, `log_with`,
//! `layer()`, capacité `logs`, drop-oldest, push hors runtime et depuis une
//! tâche tokio.
//!
//! Le registre est global au processus : un seul test, qui déroule les étapes
//! dans l'ordre, évite toute course entre elles.

#![cfg(debug_assertions)]

use std::time::Duration;

use deveye_sdk::{log, log_with, LogLevel, LOG_BUFFER_CAPACITY};
use serde_json::{json, Value};

const PORT: u16 = 31602;

fn url(path: &str) -> String {
    format!("http://127.0.0.1:{PORT}{path}")
}

async fn poll(http: &reqwest::Client) -> Vec<Value> {
    let body: Value = http
        .get(url("/devtools/logs/poll"))
        .send()
        .await
        .expect("GET /devtools/logs/poll")
        .json()
        .await
        .expect("JSON");
    body["events"].as_array().cloned().unwrap_or_default()
}

async fn capabilities(http: &reqwest::Client) -> Vec<String> {
    let info: Value = http
        .get(url("/devtools/info"))
        .send()
        .await
        .expect("GET /devtools/info")
        .json()
        .await
        .expect("JSON");
    info["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn push_logs_without_global_subscriber() {
    deveye_sdk::init(deveye_sdk::Config::with_port(PORT));
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", PORT))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let http = reqwest::Client::new();

    // Rien d'armé : pas de capacité `logs` annoncée.
    assert!(!capabilities(&http).await.contains(&"logs".to_string()));

    // 1. Trois pushs, dans l'ordre, sans aucun abonné tracing.
    log(LogLevel::Info, "app::a", "un");
    log_with(
        LogLevel::Warn,
        "app::b",
        "deux",
        json!({ "password": "pw-secret", "user": "alice" }),
    );
    log(LogLevel::Error, "app::c", String::from("trois"));
    // Sous le seuil par défaut (info) : filtré.
    log(LogLevel::Debug, "app::d", "bruit");

    assert!(capabilities(&http).await.contains(&"logs".to_string()));

    let events = poll(&http).await;
    let messages: Vec<_> = events.iter().map(|e| e["message"].as_str()).collect();
    assert_eq!(messages, [Some("un"), Some("deux"), Some("trois")]);
    assert_eq!(events[0]["target"], "app::a");
    assert_eq!(events[1]["level"], "warn");
    assert_eq!(events[1]["fields"]["password"], deveye_sdk::REDACTED);
    assert_eq!(events[1]["fields"]["user"], "alice");
    assert!(events[0]["timestamp"].as_str().unwrap().ends_with('Z'));

    // 2. Le poll draine : le second est vide.
    assert!(poll(&http).await.is_empty());

    // 3. Push depuis un fil nu (hors de tout runtime) et depuis une tâche tokio.
    std::thread::spawn(|| log(LogLevel::Info, "thread", "fil nu"))
        .join()
        .expect("le push depuis un fil nu ne doit pas paniquer");
    tokio::spawn(async { log(LogLevel::Info, "task", "tache tokio") })
        .await
        .expect("le push depuis une tâche tokio ne doit pas paniquer");
    let events = poll(&http).await;
    let messages: Vec<_> = events.iter().map(|e| e["message"].as_str()).collect();
    assert_eq!(messages, [Some("fil nu"), Some("tache tokio")]);

    // 4. 1500 pushs : les 1024 plus récents sont conservés, dans l'ordre.
    for i in 0..1500 {
        log(LogLevel::Info, "bulk", format!("m{i}"));
    }
    let events = poll(&http).await;
    assert_eq!(events.len(), LOG_BUFFER_CAPACITY);
    assert_eq!(events[0]["message"], "m476");
    assert_eq!(events[LOG_BUFFER_CAPACITY - 1]["message"], "m1499");

    // 5. layer() composé avec un abonné local (pas le global).
    #[cfg(feature = "tracing-layer")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        let subscriber = tracing_subscriber::registry().with(deveye_sdk::layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(token = "tok-secret", user = "bob", "via layer");
            tracing::debug!("filtré par le seuil");
        });
        let events = poll(&http).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["message"], "via layer");
        assert_eq!(events[0]["fields"]["token"], deveye_sdk::REDACTED);
        assert_eq!(events[0]["fields"]["user"], "bob");
    }
}
