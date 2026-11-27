//! Tests de bout en bout du binaire : version, validation des flags, doctor.
//! Aucun serveur n'est lancé et rien n'est écrit : seuls les codes de sortie et
//! les messages comptent.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deveye-server"))
        .args(args)
        .output()
        .expect("lancement du binaire")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn version_prints_crate_version_commit_and_protocols() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.starts_with("deveye-server "), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    assert!(text.contains("commit"), "{text}");
    assert!(
        text.contains("protocole SDK (wire protocol) : 1."),
        "{text}"
    );
    assert!(text.contains("protocole MCP : "), "{text}");
    // `-V` : la première ligne seulement.
    let short = stdout(&run(&["-V"]));
    assert_eq!(short.lines().count(), 1, "{short}");
}

#[test]
fn no_mode_is_a_usage_error() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--serve-mcp"), "{}", stderr(&out));
}

#[test]
fn ignored_flags_are_refused_at_startup() {
    let out = run(&[
        "--target",
        "sdk",
        "--cdp-url",
        "http://localhost:9222",
        "--serve-mcp",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(err.contains("--cdp-url") && err.contains("sdk"), "{err}");
}

#[test]
fn doctor_fails_when_a_blocking_prerequisite_is_missing() {
    // Port 1 : aucun SDK ; la cible sdk l'exige.
    let out = run(&[
        "doctor",
        "--target",
        "sdk",
        "--sdk-url",
        "http://127.0.0.1:1",
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(
        text.contains("[ÉCHEC]") && text.contains("Conseil :"),
        "{text}"
    );
    assert!(text.contains("Résumé :"), "{text}");
}

#[test]
fn doctor_does_not_fail_on_optional_gaps() {
    // Cible auto : ni CDP ni SDK ne sont bloquants.
    let out = run(&[
        "doctor",
        "--cdp-url",
        "http://127.0.0.1:1",
        "--sdk-url",
        "http://127.0.0.1:1",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(stdout(&out).contains("[ATTENTION]"));
}

#[test]
fn doctor_rejects_flags_ignored_by_the_target() {
    let out = run(&["doctor", "--target", "kwin-eis", "--cdp-accept-blank"]);
    assert_eq!(out.status.code(), Some(2));
}

/// Faux SDK : annonce `ui.widgets/1` et sert l'arbre donné sur la route de commande.
fn fake_sdk(tree: &'static str) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port éphémère");
    let port = listener.local_addr().expect("adresse").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                // La sonde TCP ouvre d'abord une connexion muette.
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let body = if request.starts_with("GET /devtools/info") {
                    r#"{"sdk_version":"1.1.0","protocol_version":"1.0","capabilities":["state","commands","info","eval","ui.widgets/1"]}"#
                } else {
                    tree
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     x-deveye-protocol-version: 1.0\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            });
        }
    });
    port
}

#[test]
fn doctor_fails_on_duplicate_widget_ids() {
    let port = fake_sdk(
        r#"{"schema":"deveye.ui/1","window":{"width":100,"height":100},"widgets":[
            {"id":"dup","role":"button","rect":[0,0,10,10]},
            {"id":"dup","role":"label","rect":[0,20,10,10]}]}"#,
    );
    let out = run(&[
        "doctor",
        "--target",
        "sdk",
        "--sdk-url",
        &format!("http://127.0.0.1:{port}"),
    ]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("Arbre ui.widgets") && text.contains("dup"),
        "{text}"
    );
}

#[test]
fn doctor_accepts_a_valid_widget_tree() {
    let port = fake_sdk(
        r#"{"schema":"deveye.ui/1","window":{"width":100,"height":100},"widgets":[
            {"id":"a","role":"button","rect":[0,0,10,10]}]}"#,
    );
    let out = run(&[
        "doctor",
        "--target",
        "sdk",
        "--sdk-url",
        &format!("http://127.0.0.1:{port}"),
    ]);
    let text = stdout(&out);
    assert!(text.contains("1 widget(s), ids uniques"), "{text}");
}
