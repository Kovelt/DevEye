//! Adapter DevEye implémentant `CommandRouter` au-dessus du wire protocol du
//! SDK opt-in : `POST /devtools/command` (invoke) et `GET /devtools/commands`
//! (list). Permet à un agent de piloter l'app par sa **logique métier** plutôt
//! que par l'UI — l'app déclare ses routes côté SDK via `expose_route`, le
//! client les découvre puis les déclenche.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use async_trait::async_trait;

use deveye_core::CommandDescriptor;
use deveye_ports::{CommandRouter, PortError, PortResult};

/// `CommandRouter` parlant au SDK opt-in via HTTP local.
#[derive(Clone, Debug)]
pub struct SdkCommandRouter {
    base_url: String,
    http: reqwest::Client,
}

impl SdkCommandRouter {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: deveye_sdk_client::client(),
        }
    }
}

#[async_trait]
impl CommandRouter for SdkCommandRouter {
    async fn invoke(
        &self,
        route: &str,
        payload: serde_json::Value,
    ) -> PortResult<serde_json::Value> {
        let url = format!("{}/devtools/command", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "route": route, "payload": payload }))
            .send()
            .await
            .map_err(|e| PortError::Network(format!("POST {url} : {e}")))?;
        let status = resp.status();
        // Lire le statut AVANT de décoder : une réponse d'erreur non-JSON
        // (proxy, corps vide) ne doit pas masquer le vrai code en erreur de
        // désérialisation.
        let text = resp
            .text()
            .await
            .map_err(|e| PortError::Network(format!("lecture reponse {url} : {e}")))?;
        if !status.is_success() {
            // 404 = route inconnue ; le corps porte `{ error }` (sinon texte brut).
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| format!("statut {status} : {text}"));
            return Err(PortError::Route {
                route: route.to_string(),
                message,
            });
        }
        serde_json::from_str(&text).map_err(|e| PortError::Route {
            route: route.to_string(),
            message: format!("décode réponse commande : {e}"),
        })
    }

    async fn list(&self) -> PortResult<Vec<CommandDescriptor>> {
        let url = format!("{}/devtools/commands", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| PortError::Network(format!("GET {url} : {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            // Un échec de listing est un problème de liaison/SDK, pas une
            // route inconnue : Network (et non Route) pour que les
            // compositions à repli puissent basculer.
            return Err(PortError::Network(format!("GET {url} : statut {status}")));
        }
        resp.json()
            .await
            .map_err(|e| PortError::Network(format!("decode JSON commandes {url} : {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::SdkCommandRouter;
    use deveye_ports::{CommandRouter, PortError};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ---- invoke ---------------------------------------------------------------

    /// POST /devtools/command 200 + corps JSON => valeur retournee.
    #[tokio::test]
    async fn invoke_ok_returns_json_value() {
        let server = MockServer::start().await;
        let expected_body = serde_json::json!({ "route": "say-hello", "payload": {} });
        Mock::given(method("POST"))
            .and(path("/devtools/command"))
            .and(body_json(expected_body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .mount(&server)
            .await;

        let adapter = SdkCommandRouter::new(server.uri());
        let result = adapter
            .invoke("say-hello", serde_json::json!({}))
            .await
            .expect("should succeed on 200");
        assert_eq!(result, serde_json::json!({ "ok": true }));
    }

    /// POST /devtools/command 404 + `{ "error": "..." }` => PortError::Route
    /// avec le message extrait du champ `error`.
    #[tokio::test]
    async fn invoke_404_json_error_field_is_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/command"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "error": "route inconnue" })),
            )
            .mount(&server)
            .await;

        let adapter = SdkCommandRouter::new(server.uri());
        let err = adapter
            .invoke("unknown", serde_json::json!(null))
            .await
            .expect_err("should fail on 404");
        match err {
            PortError::Route { route, message } => {
                assert_eq!(route, "unknown");
                assert_eq!(message, "route inconnue");
            }
            other => panic!("expected PortError::Route, got {other:?}"),
        }
    }

    /// POST /devtools/command 500 corps texte brut => PortError::Route
    /// (message contient le statut et le corps).
    #[tokio::test]
    async fn invoke_500_plain_text_returns_route_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/command"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let adapter = SdkCommandRouter::new(server.uri());
        let err = adapter
            .invoke("any", serde_json::json!(null))
            .await
            .expect_err("should fail on 500");
        assert!(
            matches!(err, PortError::Route { .. }),
            "expected PortError::Route, got {err:?}"
        );
    }

    // ---- list -----------------------------------------------------------------

    /// GET /devtools/commands 200 + tableau JSON => Vec<CommandDescriptor> deserialise.
    #[tokio::test]
    async fn list_ok_returns_descriptors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "name": "say-hello", "description": "Salue l'utilisateur" },
                { "name": "clear-data", "description": "Efface les donnees" }
            ])))
            .mount(&server)
            .await;

        let adapter = SdkCommandRouter::new(server.uri());
        let list = adapter.list().await.expect("should succeed on 200");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "say-hello");
        assert_eq!(list[1].name, "clear-data");
    }

    /// GET /devtools/commands 503 => PortError::Network (probleme de liaison SDK).
    #[tokio::test]
    async fn list_503_returns_network_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/commands"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let adapter = SdkCommandRouter::new(server.uri());
        let err = adapter.list().await.expect_err("should fail on 503");
        assert!(
            matches!(err, PortError::Network(_)),
            "expected PortError::Network, got {err:?}"
        );
    }
}
