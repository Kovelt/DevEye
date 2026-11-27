//! Lecture de `X-DevEye-Protocol-Version` et de `GET /devtools/info` par le
//! client, contre un faux SDK (wiremock).

use deveye_ports::PortError;
use deveye_sdk_client::{
    check_protocol_version, client, fetch_info, PROTOCOL_VERSION_HEADER, SUPPORTED_PROTOCOL_VERSION,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server_answering(version: Option<&str>) -> MockServer {
    let server = MockServer::start().await;
    let mut template = ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "sdk_version": "1.0.0",
        "protocol_version": "1.0",
        "capabilities": ["state", "commands", "info"]
    }));
    if let Some(version) = version {
        template = template.insert_header(PROTOCOL_VERSION_HEADER, version);
    }
    Mock::given(method("GET"))
        .and(path("/devtools/info"))
        .respond_with(template)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn matching_version_is_accepted() {
    let server = server_answering(Some(SUPPORTED_PROTOCOL_VERSION)).await;
    let response = client()
        .get(format!("{}/devtools/info", server.uri()))
        .send()
        .await
        .unwrap();
    assert!(check_protocol_version(&response).is_ok());
}

#[tokio::test]
async fn absent_header_only_warns() {
    let server = server_answering(None).await;
    let response = client()
        .get(format!("{}/devtools/info", server.uri()))
        .send()
        .await
        .unwrap();
    assert!(check_protocol_version(&response).is_ok());
}

#[tokio::test]
async fn different_minor_only_warns() {
    let server = server_answering(Some("1.7")).await;
    let response = client()
        .get(format!("{}/devtools/info", server.uri()))
        .send()
        .await
        .unwrap();
    assert!(check_protocol_version(&response).is_ok());
}

#[tokio::test]
async fn different_major_is_a_protocol_error() {
    let server = server_answering(Some("2.0")).await;
    let response = client()
        .get(format!("{}/devtools/info", server.uri()))
        .send()
        .await
        .unwrap();
    match check_protocol_version(&response) {
        Err(PortError::Protocol(message)) => {
            assert!(message.contains("2.0"), "message : {message}");
        }
        other => panic!("erreur de protocole attendue, recu {other:?}"),
    }
}

#[tokio::test]
async fn fetch_info_reads_the_payload() {
    let server = server_answering(Some("1.0")).await;
    let info = fetch_info(&client(), &server.uri()).await.unwrap();
    assert_eq!(info.sdk_version, "1.0.0");
    assert_eq!(info.protocol_version, "1.0");
    assert_eq!(info.capabilities, vec!["state", "commands", "info"]);
}

#[tokio::test]
async fn fetch_info_refuses_a_foreign_major() {
    let server = server_answering(Some("2.0")).await;
    let err = fetch_info(&client(), &server.uri()).await.unwrap_err();
    assert!(matches!(err, PortError::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn fetch_info_reports_a_pre_1_0_sdk_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/devtools/info"))
        .respond_with(ResponseTemplate::new(404).insert_header(PROTOCOL_VERSION_HEADER, "1"))
        .mount(&server)
        .await;
    let err = fetch_info(&client(), &server.uri()).await.unwrap_err();
    assert!(matches!(err, PortError::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn fetch_info_maps_unreachable_sdk_to_network() {
    // Port fermé : aucun serveur ne l'écoute.
    let err = fetch_info(&client(), "http://127.0.0.1:1")
        .await
        .unwrap_err();
    assert!(matches!(err, PortError::Network(_)), "{err:?}");
}
