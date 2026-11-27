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

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
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
                .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE),
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

/// Le binaire en mode MCP, piloté ligne à ligne (un message JSON par ligne).
struct McpSession {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl McpSession {
    fn start(display: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deveye-server"))
            .args(["--target", "x11", "--display", display])
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
                assert!(msg.get("error").is_none(), "`{method}` en erreur : {msg}");
                return msg["result"].clone();
            }
        }
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

    // target_info décrit bien la cible x11 câblée.
    let info = mcp.call_tool("target_info", json!({}));
    assert_ne!(info["isError"], json!(true), "{info}");
    let text = info["content"][0]["text"].as_str().expect("texte JSON");
    let info: Value = serde_json::from_str(text).expect("target_info est du JSON");
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
    assert_eq!(fixture.button_presses(), vec![(1, 120, 90)]);
}
