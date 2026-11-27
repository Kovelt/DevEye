use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use deveye_ports::{JsEvaluator, PortError, PortResult};

use crate::client::CdpClient;

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
    exception: Option<RemoteException>,
}

/// L'objet levé. Une `Error` n'a pas de `value` (objet non sérialisable par
/// valeur) : son texte et sa trace sont dans `description`
/// (« ReferenceError: x is not defined\n    at <anonymous>:1:1 »). Un
/// `throw "texte"` porte au contraire sa `value`.
#[derive(Debug, Default, Deserialize)]
struct RemoteException {
    #[serde(default)]
    value: Value,
    #[serde(default)]
    description: Option<String>,
}

impl ExceptionDetails {
    /// Message lisible et trace éventuelle, dans l'ordre de préférence :
    /// `description` (première ligne = message, le tout = trace si elle a
    /// plusieurs lignes), puis la valeur levée, puis le `text` générique
    /// (« Uncaught »).
    fn into_error(self) -> PortError {
        let thrown = self.exception.unwrap_or_default();
        if let Some(description) = thrown.description.filter(|d| !d.trim().is_empty()) {
            let message = description.lines().next().unwrap_or("").trim().to_string();
            let stack = description.contains('\n').then_some(description);
            return PortError::Script { message, stack };
        }
        let message = match thrown.value {
            Value::Null => self.text,
            Value::String(text) => text,
            other => other.to_string(),
        };
        PortError::Script {
            message,
            stack: None,
        }
    }
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

        if let Some(exception) = response.exception {
            return Err(exception.into_error());
        }

        Ok(response.result.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(json: Value) -> ExceptionDetails {
        serde_json::from_value(json).expect("exceptionDetails valide")
    }

    #[test]
    fn an_error_object_gives_its_first_line_as_message_and_the_description_as_stack() {
        let error = details(json!({
            "text": "Uncaught",
            "exception": {
                "type": "object",
                "description": "ReferenceError: inconnu is not defined\n    at <anonymous>:1:1"
            }
        }))
        .into_error();
        match error {
            PortError::Script { message, stack } => {
                assert_eq!(message, "ReferenceError: inconnu is not defined");
                assert_eq!(
                    stack.as_deref(),
                    Some("ReferenceError: inconnu is not defined\n    at <anonymous>:1:1")
                );
            }
            other => panic!("attendu Script, obtenu {other:?}"),
        }
    }

    #[test]
    fn a_single_line_description_has_no_stack() {
        let error = details(json!({
            "text": "Uncaught",
            "exception": { "description": "Erreur sans trace" }
        }))
        .into_error();
        assert!(matches!(
            error,
            PortError::Script { ref message, stack: None } if message == "Erreur sans trace"
        ));
    }

    #[test]
    fn a_thrown_value_is_the_message() {
        let error = details(json!({
            "text": "Uncaught",
            "exception": { "type": "string", "value": "boom" }
        }))
        .into_error();
        assert!(matches!(
            error,
            PortError::Script { ref message, stack: None } if message == "boom"
        ));
    }

    #[test]
    fn without_any_detail_the_generic_text_is_kept() {
        let error = details(json!({ "text": "Uncaught" })).into_error();
        assert!(matches!(
            error,
            PortError::Script { ref message, stack: None } if message == "Uncaught"
        ));
    }
}
