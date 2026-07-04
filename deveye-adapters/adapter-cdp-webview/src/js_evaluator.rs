use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use deveye_ports::{JsEvaluator, PortError, PortResult};

use crate::client::CdpClient;
use crate::error::CdpError;

/// Réponse `Runtime.evaluate` minimaliste — on ne s'intéresse qu'au
/// `result.value` (retourné par valeur grâce à `returnByValue: true`) et à
/// l'éventuel `exceptionDetails`.
#[derive(Debug, Deserialize)]
struct EvaluateResult {
    #[serde(default)]
    result: RuntimeValue,
    #[serde(default, rename = "exceptionDetails")]
    exception: Option<ExceptionDetails>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeValue {
    #[serde(default)]
    value: Value,
}

#[derive(Debug, Deserialize)]
struct ExceptionDetails {
    text: String,
    #[serde(default, rename = "exception")]
    exception: Option<RuntimeValue>,
}

#[async_trait]
impl JsEvaluator for CdpClient {
    async fn evaluate(&self, expression: &str) -> PortResult<Value> {
        let response: EvaluateResult = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    "userGesture": true,
                    // Avec awaitPromise:true, une Promise jamais résolue
                    // tiendrait l'appel (et donc le gate MCP) indéfiniment ;
                    // cette borne côté navigateur complète le timeout côté
                    // client (audit A1/C2-a) et doit rester STRICTEMENT
                    // INFÉRIEURE à CDP_RESPONSE_TIMEOUT (20 s) pour que le
                    // navigateur réponde par une erreur propre avant que le
                    // client n'abandonne (session CDP saine, pas d'entrée
                    // pending orpheline).
                    "timeout": 15000,
                }),
            )
            .await
            .map_err(PortError::from)?;

        if let Some(ex) = response.exception {
            let detail = ex
                .exception
                .and_then(|v| match v.value {
                    Value::Null => None,
                    other => Some(other.to_string()),
                })
                .unwrap_or_default();
            let message = if detail.is_empty() {
                ex.text
            } else {
                format!("{} — {}", ex.text, detail)
            };
            return Err(CdpError::Remote { code: 0, message }.into());
        }

        Ok(response.result.value)
    }
}
