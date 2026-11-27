//! Test de bout en bout du binaire : `deveye-server --target x11 --serve-mcp`
//! piloté en JSON-RPC sur stdio, contre un Xvfb PRIVÉ et une fenêtre fixture
//! créée par le test. `#[ignore]` : il lance un vrai serveur X.
//!
//! ```text
//! cargo test -p deveye-server --test e2e_mcp -- --ignored --test-threads=1
//! ```
//!
//! Isolation : display libre entre `:231` et `:239` (plage distincte de celle
//! de adapter-x11), sans écoute TCP ; le binaire refuse de toute façon tout
//! display < 100 ou égal à `$DISPLAY`. Xvfb est arrêté par SIGTERM à la fin
//! (jamais de `pkill -f`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adapter_mcp_server::{EXPECTED_TOOL_COUNT, MCP_PROTOCOL_VERSION};
use serde_json::{json, Value};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::COPY_DEPTH_FROM_PARENT;

const TITLE: &str = "DevEye E2E Fixture";
/// Taille de la fenêtre fixture (rouge, 400 x 300).
const SIZE: (u16, u16) = (400, 300);
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Xvfb privé, arrêté proprement au `Drop`.
struct Xvfb {
    child: Child,
    number: u32,
}

fn display_is_free(n: u32) -> bool {
    !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
        && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
}

impl Xvfb {
    fn start() -> Self {
        // Entre `display_is_free` et le lancement de Xvfb, un autre test peut
        // prendre le numéro : on essaie le suivant si Xvfb quitte aussitôt, et
        // on garde son stderr pour que l'échec reste diagnostiquable.
        let mut last_failure = String::new();
        for number in (231..=239).filter(|&n| display_is_free(n)) {
            let mut child = Command::new("Xvfb")
                .arg(format!(":{number}"))
                .args(["-nolisten", "tcp", "-screen", "0", "1400x1000x24"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("lancement de Xvfb");
            let display = format!(":{number}");
            let deadline = Instant::now() + Duration::from_secs(10);
            let ready = loop {
                if RustConnection::connect(Some(&display)).is_ok() {
                    break true;
                }
                if let Some(status) = child.try_wait().expect("état de Xvfb") {
                    let mut stderr = String::new();
                    if let Some(mut pipe) = child.stderr.take() {
                        let _ = std::io::Read::read_to_string(&mut pipe, &mut stderr);
                    }
                    last_failure = format!("Xvfb {display} a quitté ({status}) : {stderr}");
                    break false;
                }
                assert!(Instant::now() < deadline, "Xvfb {display} ne répond pas");
                std::thread::sleep(Duration::from_millis(50));
            };
            if ready {
                // stderr n'est plus lu : le libérer évite de remplir le tube.
                drop(child.stderr.take());
                return Xvfb { child, number };
            }
        }
        panic!("aucun display Xvfb utilisable entre :231 et :239 ({last_failure})");
    }

    fn display(&self) -> String {
        format!(":{}", self.number)
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg(self.child.id().to_string())
            .status();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().ok().flatten().is_none() {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let n = self.number;
        let _ = std::fs::remove_file(format!("/tmp/.X11-unix/X{n}"));
        let _ = std::fs::remove_file(format!("/tmp/.X{n}-lock"));
    }
}

/// Fenêtre fixture : fond rouge, titrée, qui journalise les appuis de bouton.
struct Fixture {
    conn: RustConnection,
}

impl Fixture {
    fn new(display: &str) -> Self {
        let (conn, screen) = RustConnection::connect(Some(display)).expect("connexion fixture");
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().expect("id de fenêtre");
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            root,
            50,
            40,
            SIZE.0,
            SIZE.1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(0x00ff_0000)
                .event_mask(
                    EventMask::BUTTON_PRESS
                        | EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::EXPOSURE,
                ),
        )
        .expect("création de la fenêtre");
        let utf8 = conn
            .intern_atom(false, b"UTF8_STRING")
            .expect("atome")
            .reply()
            .expect("réponse")
            .atom;
        let net = conn
            .intern_atom(false, b"_NET_WM_NAME")
            .expect("atome")
            .reply()
            .expect("réponse")
            .atom;
        conn.change_property8(PropMode::REPLACE, win, net, utf8, TITLE.as_bytes())
            .expect("titre");
        conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"fixture\0DevEyeFixture\0",
        )
        .expect("classe");
        conn.map_window(win).expect("map");
        conn.flush().expect("flush");
        let f = Fixture { conn };
        f.sync();
        f
    }

    fn sync(&self) {
        self.conn
            .get_input_focus()
            .expect("sync")
            .reply()
            .expect("réponse sync");
    }

    /// Événements d'entrée reçus depuis le dernier appel : `(masques `state` des
    /// appuis de bouton, horodatages des KeyPress, horodatages des KeyRelease)`.
    fn input_events(&self) -> (Vec<u16>, Vec<u32>, Vec<u32>) {
        self.sync();
        let (mut buttons, mut presses, mut releases) = (vec![], vec![], vec![]);
        while let Some(e) = self.conn.poll_for_event().expect("événement") {
            match e {
                Event::ButtonPress(b) => buttons.push(u16::from(b.state)),
                Event::KeyPress(k) => presses.push(k.time),
                Event::KeyRelease(k) => releases.push(k.time),
                _ => {}
            }
        }
        (buttons, presses, releases)
    }

    /// Appuis de bouton `(bouton, x, y)` reçus depuis le dernier appel.
    fn button_presses(&self) -> Vec<(u8, i16, i16)> {
        self.sync();
        let mut out = Vec::new();
        while let Some(e) = self.conn.poll_for_event().expect("événement") {
            if let Event::ButtonPress(b) = e {
                out.push((b.detail, b.event_x, b.event_y));
            }
        }
        out
    }
}

/// SDK factice : un serveur HTTP minimal qui sert la route `ui.widgets` (arbre
/// `deveye.ui/1` modifiable par le test, comme une app qui re-met en page) et
/// répond 404 à tout le reste. Aucune dépendance : `std::net` seulement.
struct FakeSdk {
    url: String,
    tree: Arc<Mutex<Option<Value>>>,
}

impl FakeSdk {
    fn start(tree: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port local");
        let url = format!("http://{}", listener.local_addr().expect("adresse"));
        let tree = Arc::new(Mutex::new(Some(tree)));
        let shared = tree.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let tree = shared.clone();
                std::thread::spawn(move || Self::serve(&mut stream, &tree));
            }
        });
        FakeSdk { url, tree }
    }

    fn set(&self, tree: Value) {
        *self.tree.lock().expect("verrou") = Some(tree);
    }

    fn serve(stream: &mut std::net::TcpStream, tree: &Mutex<Option<Value>>) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone du flux"));
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return; // sonde TCP muette
        }
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
        let asked: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let served = if request_line.starts_with("POST /devtools/command")
            && asked["route"] == "ui.widgets"
        {
            tree.lock().expect("verrou").clone()
        } else {
            None
        };
        let (status, payload) = match served {
            Some(tree) => ("200 OK", tree),
            None => ("404 Not Found", json!({ "error": "route inconnue" })),
        };
        let payload = payload.to_string();
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\nx-deveye-protocol-version: 1.0\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(reply.as_bytes());
    }
}

/// Le binaire en mode MCP, piloté ligne à ligne (un message JSON par ligne).
struct McpSession {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl McpSession {
    fn start(display: &str) -> Self {
        Self::start_with_sdk(display, None)
    }

    /// `sdk_url` : SDK factice qui sert la route `ui.widgets` (voir [`FakeSdk`]).
    fn start_with_sdk(display: &str, sdk_url: Option<&str>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_deveye-server"));
        command.args(["--target", "x11", "--display", display]);
        if let Some(url) = sdk_url {
            command.args(["--sdk-url", url]);
        }
        let mut child = command
            .args(["--window", TITLE, "--serve-mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("lancement du binaire");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = channel();
        // Un thread dédié lit stdout : `recv_timeout` borne chaque attente.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        McpSession {
            child,
            stdin,
            lines,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").expect("écriture stdin");
        self.stdin.flush().expect("flush stdin");
    }

    /// Requête JSON-RPC : renvoie le champ `result` de la réponse de même id
    /// (les notifications intercalées sont ignorées) ; échoue sur `error`.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let msg = self.request_raw(method, params);
        assert!(msg.get("error").is_none(), "`{method}` en erreur : {msg}");
        msg["result"].clone()
    }

    /// Comme `request`, mais renvoie le message complet (`result` ou `error`).
    fn request_raw(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|e| panic!("pas de réponse à `{method}` : {e}"));
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                panic!("stdout MCP pollué par une ligne non JSON : {line}");
            };
            if msg.get("id") == Some(&json!(id)) {
                return msg;
            }
        }
    }

    /// Appel d'outil dont l'erreur JSON-RPC éventuelle est rendue au test.
    fn call_tool_raw(&mut self, name: &str, arguments: Value) -> Value {
        self.request_raw(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        // Fermer stdin termine le serveur stdio ; sinon, arrêt forcé.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "démarre un Xvfb privé et le binaire deveye-server"]
fn mcp_stdio_session_on_private_xvfb() {
    let xvfb = Xvfb::start();
    let fixture = Fixture::new(&xvfb.display());
    let mut mcp = McpSession::start(&xvfb.display());

    // Poignée de main : la version négociée est celle du serveur.
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "deveye-e2e", "version": "0" }
        }),
    );
    assert_eq!(init["protocolVersion"], MCP_PROTOCOL_VERSION, "{init}");
    assert!(init["capabilities"]["tools"].is_object(), "{init}");
    mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    // Liste d'outils : nombre exact, constante partagée avec le test de comptage.
    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools est un tableau");
    assert_eq!(tools.len(), EXPECTED_TOOL_COUNT, "nombre d'outils");
    for expected in ["screenshot", "click", "target_info"] {
        assert!(
            tools.iter().any(|t| t["name"] == expected),
            "outil `{expected}` absent"
        );
    }

    // Sorties structurées (ADR-0067) : `outputSchema` pour les outils du palier
    // A seulement ; la capture et l'évaluation JS restent en texte/image seuls.
    let has_schema = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("outil `{name}` absent"))
            .get("outputSchema")
            .is_some_and(Value::is_object)
    };
    assert!(has_schema("target_info") && has_schema("click"));
    assert!(!has_schema("screenshot") && !has_schema("eval_js") && !has_schema("dom"));

    // target_info décrit bien la cible x11 câblée ; son texte est inchangé et
    // `structuredContent` en porte le même JSON.
    let info = mcp.call_tool("target_info", json!({}));
    assert_ne!(info["isError"], json!(true), "{info}");
    let text = info["content"][0]["text"].as_str().expect("texte JSON");
    let info_structured = info["structuredContent"].clone();
    let info: Value = serde_json::from_str(text).expect("target_info est du JSON");
    assert_eq!(info_structured, info, "structuredContent diverge du texte");
    assert_eq!(info["target"], "x11", "{info}");
    assert_eq!(info["eval_js"], json!(false), "{info}");
    assert_eq!(info["list_windows"], json!(true), "{info}");
    assert_eq!(
        info["server"]["version"],
        env!("CARGO_PKG_VERSION"),
        "{info}"
    );

    // state : sans SDK ni signaux, l'outil répond par un résultat structuré
    // (jamais une erreur de protocole), qu'il soit succès ou `isError`.
    let state = mcp.call_tool("state", json!({}));
    assert!(state["content"].is_array(), "{state}");

    // Un outil non pris en charge par la cible x11 (pas de JS) : `isError`
    // structuré, pas une erreur JSON-RPC.
    let eval = mcp.call_tool("eval_js", json!({ "expression": "1 + 1" }));
    assert_eq!(eval["isError"], json!(true), "{eval}");
    let text = eval["content"][0]["text"]
        .as_str()
        .expect("texte de l'erreur");
    assert!(text.contains("unsupported"), "{text}");

    // screenshot : un bloc image PNG non vide (signature PNG en base64).
    let shot = mcp.call_tool("screenshot", json!({}));
    assert_ne!(shot["isError"], json!(true), "{shot}");
    let image = shot["content"]
        .as_array()
        .expect("contenu")
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap_or_else(|| panic!("aucun bloc image : {shot}"));
    assert_eq!(image["mimeType"], "image/png", "{image}");
    let data = image["data"].as_str().expect("données base64");
    assert!(
        data.len() > 100,
        "capture trop petite : {} octets",
        data.len()
    );
    assert!(data.starts_with("iVBORw0KGgo"), "pas une signature PNG");

    // click : sans erreur, et la fenêtre fixture reçoit l'appui au bon endroit
    // (coordonnées relatives à la fenêtre ciblée).
    let _ = fixture.button_presses();
    let click = mcp.call_tool("click", json!({ "x": 120, "y": 90 }));
    assert_ne!(click["isError"], json!(true), "{click}");
    assert_eq!(click["content"][0]["text"], r#"{"ok":true}"#, "{click}");
    assert_eq!(click["structuredContent"], json!({ "ok": true }), "{click}");
    assert_eq!(fixture.button_presses(), vec![(1, 120, 90)]);
}

/// Un client resté sur la révision 2024-11-05 (antérieure à `structuredContent`)
/// négocie sa révision et lit le texte historique des outils sans erreur.
#[test]
#[ignore = "démarre un Xvfb privé et le binaire deveye-server"]
fn mcp_stdio_session_with_the_2024_11_05_revision() {
    let xvfb = Xvfb::start();
    let _fixture = Fixture::new(&xvfb.display());
    let mut mcp = McpSession::start(&xvfb.display());

    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "deveye-e2e-legacy", "version": "0" }
        }),
    );
    assert_eq!(init["protocolVersion"], "2024-11-05", "{init}");
    mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let info = mcp.call_tool("target_info", json!({}));
    assert_ne!(info["isError"], json!(true), "{info}");
    let text = info["content"][0]["text"].as_str().expect("texte JSON");
    let parsed: Value = serde_json::from_str(text).expect("target_info est du JSON");
    assert_eq!(parsed["target"], "x11", "{parsed}");
}

/// Arbre `deveye.ui/1` de la fenêtre fixture (400 x 300) : un bouton « Enregistrer »,
/// un champ, un bouton hors vue, au gré de `save_rect`.
fn widget_tree(save_rect: [i32; 4], with_save: bool) -> Value {
    let mut widgets = vec![
        json!({ "id": "root", "role": "window", "label": "Fixture", "rect": [0, 0, 400, 300] }),
        json!({ "id": "name", "parent": "root", "role": "textbox", "label": "Nom",
                "value": "alice", "rect": [10, 20, 200, 24] }),
        json!({ "id": "off", "parent": "root", "role": "button", "label": "Hors vue",
                "rect": [10, 900, 80, 30] }),
    ];
    if with_save {
        widgets.push(json!({ "id": "save", "parent": "root", "role": "button",
            "label": "Enregistrer", "rect": save_rect }));
    }
    json!({
        "schema": "deveye.ui/1",
        "window": { "width": 400, "height": 300 },
        "widgets": widgets,
    })
}

/// Critères 1, 2 et 5 de la 1.2 : sous Xvfb, `snapshot` liste les `w:<id>`, `act`
/// clique au centre REMESURÉ (le rect a changé depuis l'instantané), les références
/// sont stables d'un instantané à l'autre, et un id retiré donne `stale_ref` avec un
/// `hint`, sans aucun événement émis. Mesure aussi la taille du snapshot (critère 6).
#[test]
#[ignore = "démarre un Xvfb privé, un SDK factice et le binaire deveye-server"]
fn snapshot_and_act_by_reference_on_private_xvfb() {
    let xvfb = Xvfb::start();
    let fixture = Fixture::new(&xvfb.display());
    let sdk = FakeSdk::start(widget_tree([100, 80, 80, 30], true));
    let mut mcp = McpSession::start_with_sdk(&xvfb.display(), Some(&sdk.url));
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "deveye-e2e-elements", "version": "0" }
        }),
    );
    mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    // Annoncé comme expérimental, hors des 45 outils stables.
    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");
    for name in ["snapshot", "act"] {
        let tool = tools.iter().find(|t| t["name"] == name).expect("outil");
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|d| d.starts_with("[expérimental]")),
            "{tool}"
        );
    }
    let info = mcp.call_tool("target_info", json!({}));
    assert_eq!(info["structuredContent"]["elements"], json!(true), "{info}");

    // snapshot : les widgets, avec leurs références `w:<id>`, en texte compact.
    let text_of = |result: &Value| {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    let first = mcp.call_tool("snapshot", json!({}));
    assert_ne!(first["isError"], json!(true), "{first}");
    let first_text = text_of(&first);
    for expected in [
        "[ref=w:root]",
        "[ref=w:name]",
        "[ref=w:save]",
        "[ref=w:off] [offscreen]",
        "= \"alice\"",
    ] {
        assert!(
            first_text.contains(expected),
            "{expected} absent de :\n{first_text}"
        );
    }
    assert!(
        first["structuredContent"].is_null(),
        "texte seul par défaut"
    );
    let second_text = text_of(&mcp.call_tool("snapshot", json!({})));
    assert_eq!(first_text, second_text, "deux snapshots, mêmes références");

    // Critère 6 : taille mesurée et consignée, sans seuil bloquant.
    let json = mcp.call_tool("snapshot", json!({ "format": "json" }));
    assert_eq!(
        json["structuredContent"]["nodes"][0]["ref"], "w:root",
        "{json}"
    );
    let dom = mcp.call_tool("dom", json!({}));
    eprintln!(
        "MESURE (4 widgets) : snapshot texte {} octets, snapshot json {} octets, dom {} octets",
        first_text.len(),
        text_of(&json).len(),
        text_of(&dom).len()
    );

    // L'app re-met en page entre l'instantané et l'action : act clique au centre frais.
    sdk.set(widget_tree([200, 150, 80, 30], true));
    let _ = fixture.button_presses();
    let click = mcp.call_tool("act", json!({ "ref": "w:save", "action": "click" }));
    assert_ne!(click["isError"], json!(true), "{click}");
    let body: Value = serde_json::from_str(&text_of(&click)).expect("JSON");
    assert_eq!(
        (body["x"].as_i64(), body["y"].as_i64()),
        (Some(240), Some(165)),
        "{body}"
    );
    assert_eq!(fixture.button_presses(), vec![(1, 240, 165)]);

    // Un id retiré : stale_ref avec hint, aucun événement émis.
    sdk.set(widget_tree([200, 150, 80, 30], false));
    let stale = mcp.call_tool("act", json!({ "ref": "w:save", "action": "click" }));
    assert_eq!(stale["isError"], json!(true), "{stale}");
    let body: Value = serde_json::from_str(&text_of(&stale)).expect("JSON");
    assert_eq!(body["reason"], "stale_ref", "{body}");
    assert!(
        body["hint"].as_str().is_some_and(|h| !h.is_empty()),
        "{body}"
    );
    assert_eq!(fixture.button_presses(), Vec::<(u8, i16, i16)>::new());

    // Une référence inventée n'est pas une référence périmée.
    let unknown = mcp.call_tool("act", json!({ "ref": "w:jamais", "action": "click" }));
    let body: Value = serde_json::from_str(&text_of(&unknown)).expect("JSON");
    assert_eq!(body["reason"], "unknown_ref", "{body}");

    // Hors vue : refusé sans émission.
    let off = mcp.call_tool("act", json!({ "ref": "w:off", "action": "click" }));
    let body: Value = serde_json::from_str(&text_of(&off)).expect("JSON");
    assert_eq!(body["reason"], "offscreen", "{body}");
    assert_eq!(fixture.button_presses(), Vec::<(u8, i16, i16)>::new());

    // Mesure sur un arbre plus représentatif (liste de 50 lignes et 10 contrôles).
    let mut big = widget_tree([200, 150, 80, 30], true);
    for i in 0..50 {
        big["widgets"].as_array_mut().expect("widgets").push(json!({
            "id": format!("row/{i}"), "parent": "root", "role": "listitem",
            "label": format!("Ligne numéro {i}"), "actions": ["click"],
            "rect": [10, 20 + (i % 10) * 20, 300, 18]
        }));
    }
    sdk.set(big);
    let big_snapshot = text_of(&mcp.call_tool("snapshot", json!({})));
    let big_dom = text_of(&mcp.call_tool("dom", json!({})));
    eprintln!(
        "MESURE (~55 widgets) : snapshot texte {} octets, dom {} octets ({} %)",
        big_snapshot.len(),
        big_dom.len(),
        big_snapshot.len() * 100 / big_dom.len().max(1)
    );
    sdk.set(widget_tree([200, 150, 80, 30], false));

    // observe n'ajoute la section qu'à la demande.
    let observe = mcp.call_tool("observe", json!({ "include": ["elements"] }));
    let body: Value = serde_json::from_str(
        observe["content"]
            .as_array()
            .and_then(|c| c.iter().find(|b| b["type"] == "text"))
            .and_then(|b| b["text"].as_str())
            .expect("texte d'observe"),
    )
    .expect("JSON");
    assert_eq!(
        body["sections"]["elements"]["available"],
        json!(true),
        "{body}"
    );
}

/// Critère 5 : sans route `ui.widgets`, `snapshot` répond `unsupported` avec un hint
/// qui dit comment faire (exposer la route, ou passer par screenshot et click).
#[test]
#[ignore = "démarre un Xvfb privé et le binaire deveye-server"]
fn snapshot_without_a_widget_route_is_unsupported_with_a_hint() {
    let xvfb = Xvfb::start();
    let _fixture = Fixture::new(&xvfb.display());
    let sdk = FakeSdk::start(Value::Null);
    *sdk.tree.lock().expect("verrou") = None;
    let mut mcp = McpSession::start_with_sdk(&xvfb.display(), Some(&sdk.url));
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "deveye-e2e-elements", "version": "0" }
        }),
    );
    mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    let result = mcp.call_tool("snapshot", json!({}));
    assert_eq!(result["isError"], json!(true), "{result}");
    let body: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().expect("texte")).expect("JSON");
    assert_eq!(body["reason"], "unsupported", "{body}");
    let hint = body["hint"].as_str().expect("hint");
    assert!(
        hint.contains("ui.widgets") && hint.contains("screenshot"),
        "{hint}"
    );
}

/// Modificateurs d'entrée par MCP (ADR-0075) : Shift+clic produit `ShiftMask` côté
/// X, `key_hold` espace appui et relâchement de la durée demandée, un nom de
/// modificateur inconnu et une durée au-delà du plafond sont refusés sans rien
/// émettre.
#[test]
#[ignore = "démarre un Xvfb privé et le binaire deveye-server"]
fn modifiers_and_key_hold_through_mcp_on_private_xvfb() {
    let xvfb = Xvfb::start();
    let fixture = Fixture::new(&xvfb.display());
    let mut mcp = McpSession::start(&xvfb.display());
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "deveye-e2e-modifiers", "version": "0" }
        }),
    );
    mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    // Annotation de key_hold : destructif, non idempotent.
    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");
    let hold = tools
        .iter()
        .find(|t| t["name"] == "key_hold")
        .expect("key_hold absent");
    assert_eq!(
        hold["annotations"]["destructiveHint"],
        json!(true),
        "{hold}"
    );
    assert_eq!(
        hold["annotations"]["idempotentHint"],
        json!(false),
        "{hold}"
    );

    // Clic nu : pas de ShiftMask ; Shift+clic : ShiftMask (1).
    let _ = fixture.input_events();
    let result = mcp.call_tool("click", json!({ "x": 100, "y": 80 }));
    assert_ne!(result["isError"], json!(true), "{result}");
    let (plain, ..) = fixture.input_events();
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0] & 1, 0, "clic nu avec ShiftMask : {plain:?}");

    let result = mcp.call_tool(
        "click",
        json!({ "x": 100, "y": 80, "modifiers": ["Shift"] }),
    );
    assert_ne!(result["isError"], json!(true), "{result}");
    assert_eq!(
        result["structuredContent"],
        json!({ "ok": true }),
        "{result}"
    );
    let (shifted, ..) = fixture.input_events();
    assert_eq!(shifted.len(), 1);
    assert_eq!(shifted[0] & 1, 1, "ShiftMask attendu : {shifted:?}");

    // Ctrl+glisser et Alt+molette passent aussi, sans erreur.
    for (tool, args) in [
        (
            "drag_drop",
            json!({ "from_x": 20, "from_y": 20, "to_x": 120, "to_y": 90, "modifiers": ["ctrl"] }),
        ),
        (
            "scroll",
            json!({ "x": 50, "y": 50, "dy": 120, "modifiers": ["alt"] }),
        ),
    ] {
        let r = mcp.call_tool(tool, args);
        assert_ne!(r["isError"], json!(true), "{tool} : {r}");
    }
    let _ = fixture.input_events();

    // Nom inconnu : erreur de protocole invalid_params, rien n'est émis.
    let refused = mcp.call_tool_raw(
        "click",
        json!({ "x": 100, "y": 80, "modifiers": ["ctrl", "hyper"] }),
    );
    assert_eq!(refused["error"]["code"], json!(-32602), "{refused}");
    let (buttons, presses, _) = fixture.input_events();
    assert!(
        buttons.is_empty() && presses.is_empty(),
        "rien ne devait partir"
    );

    // key_hold : appui puis relâchement espacés d'au moins 300 ms.
    let held = mcp.call_tool("key_hold", json!({ "key": "Shift_L", "duration_ms": 300 }));
    assert_ne!(held["isError"], json!(true), "{held}");
    assert_eq!(held["structuredContent"], json!({ "ok": true }), "{held}");
    let (_, presses, releases) = fixture.input_events();
    assert_eq!(presses.len(), 1, "{presses:?}");
    assert_eq!(releases.len(), 1, "{releases:?}");
    assert!(
        releases[0].wrapping_sub(presses[0]) >= 300,
        "écart de {} ms",
        releases[0].wrapping_sub(presses[0])
    );

    // Plafond dépassé et touche inconnue : refus avant toute émission.
    let too_long = mcp.call_tool_raw(
        "key_hold",
        json!({ "key": "Shift_L", "duration_ms": 20000 }),
    );
    assert_eq!(too_long["error"]["code"], json!(-32602), "{too_long}");
    let unknown = mcp.call_tool_raw(
        "key_hold",
        json!({ "key": "touche_inconnue", "duration_ms": 10 }),
    );
    assert!(
        unknown.get("error").is_some() || unknown["result"]["isError"] == json!(true),
        "{unknown}"
    );
    let (_, presses, releases) = fixture.input_events();
    assert!(presses.is_empty() && releases.is_empty());
}
