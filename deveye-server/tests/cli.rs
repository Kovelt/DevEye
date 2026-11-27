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
