//! Helper `deveye_sdk::ui` de bout en bout : route `ui.widgets` servie en HTTP,
//! capacité annoncée, observation, valeurs secrètes absentes, payload ignoré.

use std::time::Duration;

use serde_json::{json, Value};

const PORT: u16 = 31601;

fn url(path: &str) -> String {
    format!("http://127.0.0.1:{PORT}{path}")
}

async fn command(http: &reqwest::Client, payload: Value) -> Value {
    http.post(url("/devtools/command"))
        .json(&json!({ "route": "ui.widgets", "payload": payload }))
        .send()
        .await
        .expect("POST /devtools/command")
        .json()
        .await
        .expect("réponse JSON")
}

async fn capabilities(http: &reqwest::Client) -> Vec<String> {
    let info: Value = http
        .get(url("/devtools/info"))
        .send()
        .await
        .expect("GET /devtools/info")
        .json()
        .await
        .expect("info JSON");
    info["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn ui_helper_serves_the_published_tree() {
    if !cfg!(debug_assertions) {
        return;
    }
    deveye_sdk::init(deveye_sdk::Config::with_port(PORT));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let http = reqwest::Client::new();

    // Avant tout usage du helper : ni route, ni capacité.
    assert!(!capabilities(&http)
        .await
        .iter()
        .any(|c| c == "ui.widgets/1"));

    // is_observed enregistre la route (sinon personne ne demanderait l'arbre).
    assert!(!deveye_sdk::ui::is_observed());
    assert!(capabilities(&http)
        .await
        .iter()
        .any(|c| c == "ui.widgets/1"));

    // Lecture sans publication : erreur explicite, et l'app est désormais observée.
    let body = command(&http, json!({})).await;
    assert!(body["error"].is_string(), "{body}");
    assert!(deveye_sdk::ui::is_observed());

    // L'observation expire ; la publication arrive pendant la lecture suivante,
    // qui attend l'arbre frais.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert!(!deveye_sdk::ui::is_observed());
    let publisher = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(100));
        let tree = deveye_sdk::ui::builder(640, 480)
            .focus("name")
            .widget(
                deveye_sdk::ui::UiWidget::new("name", deveye_sdk::ui::UiRole::TEXTBOX)
                    .with_rect([10, 10, 200, 24])
                    .with_value("alice"),
            )
            .widget(
                deveye_sdk::ui::UiWidget::new("pw", deveye_sdk::ui::UiRole::PASSWORD)
                    .with_rect([10, 40, 200, 24])
                    .with_value("hunter2"),
            )
            .build();
        deveye_sdk::ui::publish(tree);
    });
    // Un payload de négociation inconnu est ignoré (un handler strict casserait).
    let body = command(&http, json!({ "accept_schema": [1], "max_nodes": 10 })).await;
    publisher.join().expect("publication");
    assert_eq!(body["schema"], "deveye.ui/1", "{body}");
    assert_eq!(body["widgets"][0]["value"], "alice");
    assert!(body["widgets"][1].get("value").is_none(), "{body}");
    assert!(!body.to_string().contains("hunter2"), "fuite : {body}");
    assert_eq!(body["focus"], "name");
    assert!(body["published_at_ms"].is_u64());
}
