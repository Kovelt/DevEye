//! Test d'intégration du wire protocol DevEye : lance le micro-serveur du SDK
//! et vérifie le contrat (`docs/wire-protocol.md`) bout en bout — garde Host,
//! header de version, eval sans pompe, et le cycle expose_route / command(s).
//!
//! Le SDK tourne sur son runtime dédié ; le test parle au serveur en vrai TCP
//! loopback via reqwest (dev-dependency). Port dédié pour ne pas heurter une
//! instance de dev sur le port par défaut 31337.

use std::time::Duration;

use reqwest::header::HOST;
use reqwest::StatusCode;

const PORT: u16 = 31599;

fn url(path: &str) -> String {
    format!("http://127.0.0.1:{PORT}{path}")
}

#[tokio::test]
async fn wire_protocol_contract_end_to_end() {
    // Le SDK n'ouvre de serveur qu'en `debug_assertions` (filet de sécurité
    // release de `init`) ; ce test de contrat n'a donc de sens qu'en debug.
    if !cfg!(debug_assertions) {
        return;
    }
    std::env::set_var("DEVEYE_PORT", PORT.to_string());
    deveye_sdk::init(deveye_sdk::Config::default());
    // Laisser le runtime dédié binder le port avant de cogner dessus.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let http = reqwest::Client::new();

    // 1. POST /devtools/eval sans pompe enregistrée => 503 + { error },
    //    et le header de version est présent même sur cette réponse.
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .json(&serde_json::json!({ "expression": "1+1" }))
        .send()
        .await
        .expect("POST /devtools/eval");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers()
            .get("x-deveye-protocol-version")
            .and_then(|v| v.to_str().ok()),
        Some("1"),
        "header de version absent du 503"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("error").is_some(), "503 doit porter un message");

    // 2. Host étranger => 403 (garde anti DNS-rebinding), header version inclus.
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "evil.com")
        .json(&serde_json::json!({ "expression": "1+1" }))
        .send()
        .await
        .expect("POST avec Host etranger");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        resp.headers()
            .get("x-deveye-protocol-version")
            .and_then(|v| v.to_str().ok()),
        Some("1"),
        "header de version absent du 403 de la garde"
    );

    // 3. GET /devtools/commands au démarrage => 200 + liste vide.
    let resp = http
        .get(url("/devtools/commands"))
        .header(HOST, "127.0.0.1")
        .send()
        .await
        .expect("GET /devtools/commands");
    assert_eq!(resp.status(), StatusCode::OK);
    let list: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(list, serde_json::json!([]), "aucune commande au depart");

    // 4. expose_route puis découverte + invocation.
    deveye_sdk::expose_route(
        "ping",
        "repond pong",
        |payload| serde_json::json!({ "pong": true, "echo": payload }),
    );
    tokio::time::sleep(Duration::from_millis(150)).await;

    let resp = http
        .get(url("/devtools/commands"))
        .header(HOST, "localhost")
        .send()
        .await
        .unwrap();
    let list: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        list,
        serde_json::json!([{ "name": "ping", "description": "repond pong" }])
    );

    // invocation d'une route connue : le corps est la valeur du handler, sans enveloppe.
    let resp = http
        .post(url("/devtools/command"))
        .header(HOST, "localhost")
        .json(&serde_json::json!({ "route": "ping", "payload": { "x": 1 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["pong"], serde_json::json!(true));
    assert_eq!(body["echo"]["x"], serde_json::json!(1));

    // route inconnue => 404 + corps minimal { "error": "inconnue" }. La route
    // n'est PAS repetee dans le corps : elle est deja portee par
    // `PortError::Route` cote client, dont le Display la prefixe — la repeter
    // donnait « commande X : commande inconnue : X » (audit C2-b, F2 Chronos).
    let resp = http
        .post(url("/devtools/command"))
        .header(HOST, "localhost")
        .json(&serde_json::json!({ "route": "inexistante" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], serde_json::json!("inconnue"));
    assert!(!body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("inexistante"));

    // 5. Round-trip state : set_state cote app puis GET /devtools/state.
    deveye_sdk::set_state("compteur", serde_json::json!(42));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let resp = http
        .get(url("/devtools/state"))
        .header(HOST, "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entries"]["compteur"], serde_json::json!(42));

    // 6. Chemin NOMINAL de /devtools/eval : brancher une pompe qui repond,
    //    puis verifier que le corps porte bien { value }.
    let mut requests = deveye_sdk::take_eval_requests();
    tokio::spawn(async move {
        while let Some(req) = requests.recv().await {
            // Pompe de test : repond toujours 2 (peu importe l'expression).
            let _ = req.respond.send(Ok(serde_json::json!(2)));
        }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let resp = http
        .post(url("/devtools/eval"))
        .header(HOST, "localhost")
        .json(&serde_json::json!({ "expression": "1+1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["value"],
        serde_json::json!(2),
        "eval nominal => {{ value }}"
    );
    assert!(body.get("error").is_none());

    // 7. logs/poll : injecter un log via la couche tracing puis le draîner.
    deveye_sdk::install_tracing_layer();
    tracing::info!("ping depuis le test d'integration");
    tokio::time::sleep(Duration::from_millis(120)).await;
    let resp = http
        .get(url("/devtools/logs/poll"))
        .header(HOST, "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["events"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "au moins un log doit etre draine apres l'info!"
    );
}
