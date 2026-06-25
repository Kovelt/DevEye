use async_trait::async_trait;
use serde_json::{json, Value};

use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{InteractionSink, PortResult};

use crate::client::CdpClient;

#[async_trait]
impl InteractionSink for CdpClient {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        match command {
            InteractionCommand::Click { position, button } => {
                let label = mouse_button_label(button);
                // mouseMoved préalable : déclenche mouseover/mouseenter avant
                // le press. Indispensable pour les UI dont les éléments
                // n'apparaissent qu'au hover (sidebars rétractables, menus
                // contextuels, etc.). On laisse 60 ms au framework pour
                // re-render avant le press : ordre de grandeur suffisant pour
                // Dioxus 0.7 sans gêner les usages humains.
                mouse_event(self, "mouseMoved", position, label).await?;
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                mouse_event(self, "mousePressed", position, label).await?;
                mouse_event(self, "mouseReleased", position, label).await?;
            }
            InteractionCommand::Hover { position } => {
                mouse_event(self, "mouseMoved", position, "left").await?;
            }
            InteractionCommand::Type { text } => {
                let _: Value = self
                    .call("Input.insertText", json!({ "text": text }))
                    .await
                    .map_err(Into::<deveye_ports::PortError>::into)?;
            }
            InteractionCommand::KeyPress { key } => {
                let payload = json!({
                    "type": "keyDown",
                    "key": key,
                });
                let _: Value = self
                    .call("Input.dispatchKeyEvent", payload)
                    .await
                    .map_err(Into::<deveye_ports::PortError>::into)?;
                let payload = json!({
                    "type": "keyUp",
                    "key": key,
                });
                let _: Value = self
                    .call("Input.dispatchKeyEvent", payload)
                    .await
                    .map_err(Into::<deveye_ports::PortError>::into)?;
            }
            InteractionCommand::Navigate { route } => {
                // Le routage logique est pris en charge par le SDK opt-in via
                // `window.__deveye_router__.invoke('navigate', { route })`.
                // En l'absence du SDK, l'évaluation échoue côté JS et CDP
                // remontera l'erreur — ce qui est le bon signal.
                let expression = format!(
                    "window.__deveye_router__ && window.__deveye_router__.invoke('navigate', {{ route: {} }})",
                    serde_json::to_string(&route).expect("string serialise")
                );
                let _: Value = self
                    .call(
                        "Runtime.evaluate",
                        json!({ "expression": expression, "awaitPromise": true }),
                    )
                    .await
                    .map_err(Into::<deveye_ports::PortError>::into)?;
            }
            InteractionCommand::DragDrop { from, to } => {
                // Séquence souris native : hover -> press -> déplacements
                // intermédiaires (certains frameworks exigent ≥1 mousemove
                // pendant le drag) -> release. Petits délais pour laisser le
                // framework réagir entre chaque étape.
                mouse_event(self, "mouseMoved", from, "left").await?;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                mouse_event(self, "mousePressed", from, "left").await?;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                let mid = Position::new((from.x + to.x) / 2, (from.y + to.y) / 2);
                mouse_event(self, "mouseMoved", mid, "left").await?;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                mouse_event(self, "mouseMoved", to, "left").await?;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                mouse_event(self, "mouseReleased", to, "left").await?;
            }
        }
        Ok(())
    }
}

async fn mouse_event(
    client: &CdpClient,
    kind: &str,
    position: Position,
    button: &str,
) -> PortResult<()> {
    let payload = json!({
        "type": kind,
        "x": position.x,
        "y": position.y,
        "button": button,
        "clickCount": 1,
    });
    let _: Value = client
        .call("Input.dispatchMouseEvent", payload)
        .await
        .map_err(Into::<deveye_ports::PortError>::into)?;
    Ok(())
}

fn mouse_button_label(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_label_covers_all_variants() {
        assert_eq!(mouse_button_label(MouseButton::Left), "left");
        assert_eq!(mouse_button_label(MouseButton::Right), "right");
        assert_eq!(mouse_button_label(MouseButton::Middle), "middle");
    }
}
