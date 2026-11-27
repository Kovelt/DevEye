//! Modificateurs d'entrée (`InteractionCommand::WithModifiers`, ADR-0075) dans un
//! vrai Chromium headless : le même geste décoré, par les deux chemins qui
//! atteignent une page web, doit être vu par la page avec les bons booleens
//! `shiftKey`/`ctrlKey`/`altKey`/`metaKey`.
//!
//! - CDP : masque `modifiers` de `Input.dispatchMouseEvent` (événements de confiance) ;
//! - pont SDK (`JsInteraction`, le chemin de WebKitGTK) : booleens des événements
//!   synthétiques (`isTrusted` faux).
//!
//! `#[ignore]` : il lance un navigateur.
//!
//! ```text
//! cargo test -p deveye-server --test modifiers_chromium -- --ignored --test-threads=1
//! ```
//!
//! Navigateur : `$DEVEYE_TEST_CHROMIUM`, sinon le premier trouvé parmi
//! `chromium-browser`, `chromium`, `brave-browser`, `google-chrome`.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adapter_cdp_webview::{CdpClient, TargetFilter};
use adapter_dioxus_signals::JsInteraction;
use deveye_core::{InteractionCommand, Modifier, Position};
use deveye_ports::{InteractionSink, JsEvaluator, PortError};

const FIXTURE: &str = r##"<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><title>Fixture modificateurs</title>
<style>
  body { margin: 0; }
  #zone { position: absolute; left: 20px; top: 20px; width: 300px; height: 200px; background: #ddd; }
</style></head>
<body>
  <div id="zone"></div>
  <script>
    window.log = [];
    const flags = (e) => ({ type: e.type, shift: e.shiftKey, ctrl: e.ctrlKey, alt: e.altKey,
                            meta: e.metaKey, trusted: e.isTrusted });
    const zone = document.getElementById('zone');
    for (const t of ['click', 'dblclick', 'mousedown', 'mousemove', 'wheel']) {
      zone.addEventListener(t, (e) => window.log.push(flags(e)));
    }
  </script>
</body></html>"##;

struct Chromium {
    child: Child,
    dir: PathBuf,
}

impl Drop for Chromium {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn find_browser() -> String {
    if let Ok(path) = std::env::var("DEVEYE_TEST_CHROMIUM") {
        return path;
    }
    for name in [
        "chromium-browser",
        "chromium",
        "brave-browser",
        "google-chrome",
    ] {
        let found = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
            .unwrap_or(false);
        if found {
            return name.to_string();
        }
    }
    panic!("aucun Chromium trouvé : définir DEVEYE_TEST_CHROMIUM");
}

fn start_chromium() -> (Chromium, u16, String) {
    let dir = std::env::temp_dir().join(format!("deveye-modifiers-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("répertoire temporaire");
    let page = dir.join("fixture.html");
    std::fs::write(&page, FIXTURE).expect("fixture");
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port libre");
        listener.local_addr().expect("adresse").port()
    };
    let child = Command::new(find_browser())
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--window-size=800,600",
            &format!("--remote-debugging-port={port}"),
            &format!("--user-data-dir={}", dir.join("profile").display()),
        ])
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("lancement du navigateur");
    let url = format!("file://{}", page.display());
    (Chromium { child, dir }, port, url)
}

async fn connect(port: u16) -> Arc<CdpClient> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // Le navigateur démarre sur `about:blank` ; la fixture y est chargée ensuite.
        let filter = TargetFilter {
            accept_blank: true,
            ..TargetFilter::default()
        };
        if let Ok(client) = CdpClient::connect(&format!("http://127.0.0.1:{port}"), filter).await {
            return Arc::new(client);
        }
        assert!(Instant::now() < deadline, "CDP injoignable");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for(client: &Arc<CdpClient>, expression: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(value) = client.evaluate(expression).await {
            if value == serde_json::Value::Bool(true) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "condition jamais vraie : {expression}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn last_event(client: &Arc<CdpClient>, kind: &str) -> serde_json::Value {
    let expression =
        format!("JSON.stringify(window.log.filter(e => e.type === '{kind}').slice(-1)[0] || null)");
    let raw = client.evaluate(&expression).await.expect("évaluation");
    let text = raw.as_str().expect("chaîne JSON").to_string();
    serde_json::from_str(&text).expect("JSON de l'événement")
}

async fn reset_log(client: &Arc<CdpClient>) {
    client
        .evaluate("window.log = []; true")
        .await
        .expect("reset");
}

fn decorated(modifiers: &[Modifier], inner: InteractionCommand) -> InteractionCommand {
    InteractionCommand::with_modifiers(modifiers.to_vec(), inner).expect("geste enveloppable")
}

async fn prepare() -> (Chromium, Arc<CdpClient>) {
    let (browser, port, url) = start_chromium();
    let client = connect(port).await;
    let _: serde_json::Value = client
        .call("Page.navigate", serde_json::json!({ "url": url }))
        .await
        .expect("navigation");
    wait_for(&client, "location.href.includes('fixture.html')").await;
    wait_for(&client, "document.readyState === 'complete'").await;
    wait_for(&client, "document.getElementById('zone') !== null").await;
    (browser, client)
}

#[tokio::test]
#[ignore = "lance un Chromium headless"]
async fn cdp_carries_the_modifier_mask_on_mouse_events() {
    let (_browser, client) = prepare().await;
    let at = Position::new(100, 80);

    // Controle negatif : sans modificateur, aucun booleen.
    client
        .dispatch(InteractionCommand::click(at))
        .await
        .unwrap();
    let plain = last_event(&client, "click").await;
    assert_eq!(plain["shift"], false, "{plain}");

    // Shift+clic.
    reset_log(&client).await;
    client
        .dispatch(decorated(&[Modifier::Shift], InteractionCommand::click(at)))
        .await
        .unwrap();
    let click = last_event(&client, "click").await;
    assert_eq!(click["shift"], true, "{click}");
    assert_eq!(click["ctrl"], false, "{click}");
    assert_eq!(click["trusted"], true, "{click}");

    // Ctrl+Alt+double-clic.
    reset_log(&client).await;
    client
        .dispatch(decorated(
            &[Modifier::Ctrl, Modifier::Alt],
            InteractionCommand::double_click(at),
        ))
        .await
        .unwrap();
    let dbl = last_event(&client, "dblclick").await;
    assert_eq!(
        (
            dbl["ctrl"].clone(),
            dbl["alt"].clone(),
            dbl["shift"].clone()
        ),
        (true.into(), true.into(), false.into()),
        "{dbl}"
    );

    // Meta+survol.
    reset_log(&client).await;
    client
        .dispatch(decorated(&[Modifier::Meta], InteractionCommand::hover(at)))
        .await
        .unwrap();
    let mv = last_event(&client, "mousemove").await;
    assert_eq!(mv["meta"], true, "{mv}");

    // Ctrl+molette.
    reset_log(&client).await;
    client
        .dispatch(decorated(
            &[Modifier::Ctrl],
            InteractionCommand::scroll(at, 0, 120),
        ))
        .await
        .unwrap();
    let wheel = last_event(&client, "wheel").await;
    assert_eq!(wheel["ctrl"], true, "{wheel}");

    // Ctrl+glisser : les mouvements portent le masque.
    reset_log(&client).await;
    client
        .dispatch(decorated(
            &[Modifier::Ctrl],
            InteractionCommand::drag_drop(Position::new(40, 40), Position::new(200, 150)),
        ))
        .await
        .unwrap();
    let drag = last_event(&client, "mousemove").await;
    assert_eq!(drag["ctrl"], true, "{drag}");

    // KeyHold : non supporte en CDP, sans emission.
    reset_log(&client).await;
    let err = client
        .dispatch(InteractionCommand::key_hold("Shift", 100).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, PortError::Unsupported(_)), "{err:?}");
    // Imbrication par litteral : refus sans emission.
    let nested = InteractionCommand::WithModifiers {
        modifiers: vec![Modifier::Shift],
        command: Box::new(decorated(&[Modifier::Ctrl], InteractionCommand::click(at))),
    };
    assert!(client.dispatch(nested).await.is_err());
    let count = client.evaluate("window.log.length").await.unwrap();
    assert_eq!(count, serde_json::json!(0), "rien ne doit avoir ete emis");
}

#[tokio::test]
#[ignore = "lance un Chromium headless"]
async fn the_sdk_bridge_carries_the_modifier_booleans() {
    let (_browser, client) = prepare().await;
    let evaluator: Arc<dyn JsEvaluator> = client.clone();
    let bridge = JsInteraction::new(evaluator);
    let at = Position::new(100, 80);

    bridge
        .dispatch(InteractionCommand::click(at))
        .await
        .unwrap();
    assert_eq!(last_event(&client, "click").await["shift"], false);

    reset_log(&client).await;
    bridge
        .dispatch(decorated(&[Modifier::Shift], InteractionCommand::click(at)))
        .await
        .unwrap();
    let click = last_event(&client, "click").await;
    assert_eq!(click["shift"], true, "{click}");
    assert_eq!(click["ctrl"], false, "{click}");
    // Limite documentee : un evenement synthetique n'est pas de confiance.
    assert_eq!(click["trusted"], false, "{click}");

    reset_log(&client).await;
    bridge
        .dispatch(decorated(
            &[Modifier::Ctrl, Modifier::Meta],
            InteractionCommand::double_click(at),
        ))
        .await
        .unwrap();
    let dbl = last_event(&client, "dblclick").await;
    assert_eq!(
        (dbl["ctrl"].clone(), dbl["meta"].clone()),
        (true.into(), true.into()),
        "{dbl}"
    );

    reset_log(&client).await;
    bridge
        .dispatch(decorated(&[Modifier::Alt], InteractionCommand::hover(at)))
        .await
        .unwrap();
    assert_eq!(last_event(&client, "mousemove").await["alt"], true);

    reset_log(&client).await;
    bridge
        .dispatch(decorated(
            &[Modifier::Ctrl],
            InteractionCommand::scroll(at, 0, 120),
        ))
        .await
        .unwrap();
    assert_eq!(last_event(&client, "wheel").await["ctrl"], true);

    reset_log(&client).await;
    bridge
        .dispatch(decorated(
            &[Modifier::Shift],
            InteractionCommand::drag_drop(Position::new(40, 40), Position::new(200, 150)),
        ))
        .await
        .unwrap();
    assert_eq!(last_event(&client, "mousemove").await["shift"], true);

    // KeyHold : refuse, rien n'est evalue.
    reset_log(&client).await;
    let err = bridge
        .dispatch(InteractionCommand::key_hold("Shift", 100).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, PortError::Unsupported(_)), "{err:?}");
    assert_eq!(
        client.evaluate("window.log.length").await.unwrap(),
        serde_json::json!(0)
    );
}
