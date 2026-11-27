use async_trait::async_trait;
use serde_json::{json, Value};

use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{InteractionSink, PortError, PortResult};

use crate::client::CdpClient;

#[async_trait]
impl InteractionSink for CdpClient {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        match command {
            InteractionCommand::Click { position, button } => {
                let label = mouse_button_label(button).ok_or_else(|| {
                    PortError::Unsupported(format!("bouton de souris {button:?} non géré en CDP"))
                })?;
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
                    .map_err(PortError::from)?;
            }
            InteractionCommand::KeyPress { key } => {
                let payload = json!({
                    "type": "keyDown",
                    "key": key,
                });
                let _: Value = self
                    .call("Input.dispatchKeyEvent", payload)
                    .await
                    .map_err(PortError::from)?;
                let payload = json!({
                    "type": "keyUp",
                    "key": key,
                });
                let _: Value = self
                    .call("Input.dispatchKeyEvent", payload)
                    .await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::KeyCombo { modifiers, key } => {
                // Resout chaque modificateur en (bit CDP, nom de touche). Un
                // modificateur inconnu est une erreur explicite (pas un silence).
                let mut mods: Vec<(i64, &'static str)> = Vec::with_capacity(modifiers.len());
                for m in &modifiers {
                    let resolved = cdp_modifier(m).ok_or_else(|| {
                        PortError::Interaction(format!("modificateur inconnu : {m}"))
                    })?;
                    mods.push(resolved);
                }
                let full_mask: i64 = mods.iter().fold(0, |acc, (bit, _)| acc | bit);
                // Press des modificateurs (bitmask accumule), puis press/release
                // de la touche principale (bitmask complet), puis release des
                // modificateurs en ordre inverse.
                let mut active = 0i64;
                for (bit, name) in &mods {
                    active |= bit;
                    key_event(self, "keyDown", name, active).await?;
                }
                key_event(self, "keyDown", &key, full_mask).await?;
                key_event(self, "keyUp", &key, full_mask).await?;
                for (bit, name) in mods.iter().rev() {
                    key_event(self, "keyUp", name, active).await?;
                    active &= !bit;
                }
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
                    .map_err(PortError::from)?;
            }
            InteractionCommand::Scroll { position, dx, dy } => {
                // mouseWheel ancre en (x, y) : CDP route le delta vers le bon
                // conteneur scrollable sous le point (vs window.scrollBy qui ne
                // defile que la racine).
                let payload = json!({
                    "type": "mouseWheel",
                    "x": position.x,
                    "y": position.y,
                    "deltaX": dx,
                    "deltaY": dy,
                });
                let _: Value = self
                    .call("Input.dispatchMouseEvent", payload)
                    .await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::DoubleClick { position } => {
                // mouseMoved + deux press/release ; le 2e a clickCount=2 (c'est
                // ce qui declenche l'evenement `dblclick` cote page).
                mouse_event(self, "mouseMoved", position, "left").await?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                mouse_event_cc(self, "mousePressed", position, "left", 1).await?;
                mouse_event_cc(self, "mouseReleased", position, "left", 1).await?;
                mouse_event_cc(self, "mousePressed", position, "left", 2).await?;
                mouse_event_cc(self, "mouseReleased", position, "left", 2).await?;
            }
            InteractionCommand::HandleDialog {
                accept,
                prompt_text,
            } => {
                let mut payload = json!({ "accept": accept });
                if let Some(text) = prompt_text {
                    payload["promptText"] = json!(text);
                }
                let _: Value = self
                    .call("Page.handleJavaScriptDialog", payload)
                    .await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::UploadFile { selector, paths } => {
                // DOM.getDocument -> querySelector -> setFileInputFiles : seule
                // voie valide (la politique de securite interdit de setter
                // input[type=file].value en JS).
                let doc: Value = self
                    .call("DOM.getDocument", json!({}))
                    .await
                    .map_err(PortError::from)?;
                let root_id = doc
                    .get("root")
                    .and_then(|r| r.get("nodeId"))
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        PortError::Protocol("DOM.getDocument sans root.nodeId".to_string())
                    })?;
                let found: Value = self
                    .call(
                        "DOM.querySelector",
                        json!({ "nodeId": root_id, "selector": selector }),
                    )
                    .await
                    .map_err(PortError::from)?;
                let node_id = found.get("nodeId").and_then(|v| v.as_i64()).unwrap_or(0);
                if node_id == 0 {
                    return Err(PortError::Interaction(format!(
                        "input fichier introuvable : {selector}"
                    )));
                }
                let _: Value = self
                    .call(
                        "DOM.setFileInputFiles",
                        json!({ "nodeId": node_id, "files": paths }),
                    )
                    .await
                    .map_err(PortError::from)?;
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
            InteractionCommand::Resize { width, height } => {
                let (method, params) = resize_request(width, height)?;
                let _: Value = self.call(method, params).await.map_err(PortError::from)?;
            }
            // `InteractionCommand` est `#[non_exhaustive]` : une commande
            // ajoutée par le domaine sans traduction CDP est refusée.
            // Pas de `Debug` de la commande : il embarquerait le texte tapé.
            _ => {
                return Err(PortError::Unsupported(
                    "commande non gérée par l'adapter CDP".to_string(),
                ))
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
    mouse_event_cc(client, kind, position, button, 1).await
}

/// Variante avec `clickCount` explicite (2 pour le second press/release d'un
/// double-clic — c'est ce qui declenche `dblclick`).
async fn mouse_event_cc(
    client: &CdpClient,
    kind: &str,
    position: Position,
    button: &str,
    click_count: u32,
) -> PortResult<()> {
    let payload = json!({
        "type": kind,
        "x": position.x,
        "y": position.y,
        "button": button,
        "clickCount": click_count,
    });
    let _: Value = client
        .call("Input.dispatchMouseEvent", payload)
        .await
        .map_err(PortError::from)?;
    Ok(())
}

/// `None` pour un bouton que cet adapter ne sait pas traduire (`MouseButton`
/// est `#[non_exhaustive]`).
fn mouse_button_label(button: MouseButton) -> Option<&'static str> {
    match button {
        MouseButton::Left => Some("left"),
        MouseButton::Right => Some("right"),
        MouseButton::Middle => Some("middle"),
        _ => None,
    }
}

/// Requête CDP d'un `Resize`. `deviceScaleFactor: 0` conserve le facteur
/// d'échelle réel de l'écran (ne change que les dimensions CSS). `0` x `0`
/// retire l'émulation ; un seul côté nul n'a pas de sens et est refusé.
fn resize_request(width: u32, height: u32) -> PortResult<(&'static str, Value)> {
    match (width, height) {
        (0, 0) => Ok(("Emulation.clearDeviceMetricsOverride", json!({}))),
        (0, _) | (_, 0) => Err(PortError::Interaction(format!(
            "dimensions invalides {width}x{height} : les deux côtés doivent être non nuls \
             (ou nuls ensemble pour retirer l'émulation)"
        ))),
        _ => Ok((
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 0,
                "mobile": false,
            }),
        )),
    }
}

/// Envoie un `Input.dispatchKeyEvent` (keyDown/keyUp) avec un bitmask de
/// modificateurs CDP. Utilise par `KeyCombo`.
async fn key_event(client: &CdpClient, ty: &str, key: &str, modifiers: i64) -> PortResult<()> {
    let _: Value = client
        .call(
            "Input.dispatchKeyEvent",
            json!({ "type": ty, "key": key, "modifiers": modifiers }),
        )
        .await
        .map_err(PortError::from)?;
    Ok(())
}

/// Resout un nom de modificateur (insensible a la casse) en `(bit CDP, nom de
/// touche CDP)`. Bitmask CDP : Alt=1, Ctrl=2, Meta=4, Shift=8.
fn cdp_modifier(name: &str) -> Option<(i64, &'static str)> {
    match name.to_ascii_lowercase().as_str() {
        "alt" => Some((1, "Alt")),
        "ctrl" | "control" => Some((2, "Control")),
        "meta" | "super" | "cmd" | "command" => Some((4, "Meta")),
        "shift" => Some((8, "Shift")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_label_covers_all_variants() {
        assert_eq!(mouse_button_label(MouseButton::Left), Some("left"));
        assert_eq!(mouse_button_label(MouseButton::Right), Some("right"));
        assert_eq!(mouse_button_label(MouseButton::Middle), Some("middle"));
    }

    #[test]
    fn resize_request_overrides_device_metrics_keeping_the_real_scale() {
        let (method, params) = resize_request(375, 812).expect("dimensions valides");
        assert_eq!(method, "Emulation.setDeviceMetricsOverride");
        assert_eq!(params["width"], json!(375));
        assert_eq!(params["height"], json!(812));
        assert_eq!(params["deviceScaleFactor"], json!(0));
        assert_eq!(params["mobile"], json!(false));
    }

    #[test]
    fn resize_request_zero_by_zero_clears_the_override() {
        let (method, params) = resize_request(0, 0).expect("0x0 retire l'émulation");
        assert_eq!(method, "Emulation.clearDeviceMetricsOverride");
        assert_eq!(params, json!({}));
    }

    #[test]
    fn resize_request_rejects_a_single_zero_side() {
        for (w, h) in [(0, 600), (800, 0)] {
            assert!(
                matches!(resize_request(w, h), Err(PortError::Interaction(_))),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn cdp_modifier_resolves_known_aliases_case_insensitive() {
        assert_eq!(cdp_modifier("Ctrl"), Some((2, "Control")));
        assert_eq!(cdp_modifier("control"), Some((2, "Control")));
        assert_eq!(cdp_modifier("SHIFT"), Some((8, "Shift")));
        assert_eq!(cdp_modifier("alt"), Some((1, "Alt")));
        assert_eq!(cdp_modifier("super"), Some((4, "Meta")));
        assert_eq!(cdp_modifier("cmd"), Some((4, "Meta")));
        assert_eq!(cdp_modifier("hyper"), None);
    }
}
