//! Identité du binaire : version du crate, commit de build et versions des
//! protocoles (ADR-0033). Source unique pour `--version`, `target_info`, le
//! `serverInfo` MCP et `doctor`.

use std::sync::OnceLock;

use adapter_mcp_server::{ServerVersion, MCP_PROTOCOL_VERSION};
use deveye_sdk_client::SUPPORTED_PROTOCOL_VERSION;

/// Version du crate `deveye-server`.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Commit court de la compilation, `None` hors dépôt git (`build.rs`).
pub(crate) fn commit() -> Option<&'static str> {
    let commit = env!("DEVEYE_BUILD_COMMIT");
    (!commit.is_empty()).then_some(commit)
}

/// Identité complète, au format partagé avec `adapter-mcp-server`.
pub(crate) fn server_version() -> ServerVersion {
    ServerVersion::new(
        VERSION,
        commit().map(str::to_string),
        SUPPORTED_PROTOCOL_VERSION,
    )
}

/// `0.2.0 (commit abc1234)` ou `0.2.0 (commit inconnu)`.
fn short_text(version: &str, commit: Option<&str>) -> String {
    format!("{version} (commit {})", commit.unwrap_or("inconnu"))
}

/// Texte de `--version` : une ligne, puis les versions des protocoles.
fn long_text(version: &str, commit: Option<&str>, sdk: &str, mcp: &str) -> String {
    format!(
        "{}\nprotocole SDK (wire protocol) : {sdk}\nprotocole MCP : {mcp}",
        short_text(version, commit)
    )
}

/// Pour `-V` : `&'static str` exigé par clap, construit une seule fois.
pub(crate) fn clap_short() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| short_text(VERSION, commit()))
}

/// Pour `--version`.
pub(crate) fn clap_long() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        long_text(
            VERSION,
            commit(),
            SUPPORTED_PROTOCOL_VERSION,
            MCP_PROTOCOL_VERSION,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_shows_commit_or_unknown() {
        assert_eq!(
            short_text("1.0.0", Some("abc1234")),
            "1.0.0 (commit abc1234)"
        );
        assert_eq!(short_text("1.0.0", None), "1.0.0 (commit inconnu)");
    }

    #[test]
    fn long_text_lists_both_protocols() {
        let text = long_text("1.0.0", Some("abc1234"), "1.0", "2025-11-25");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "1.0.0 (commit abc1234)");
        assert!(lines[1].ends_with("1.0") && lines[2].ends_with("2025-11-25"));
    }

    #[test]
    fn server_version_carries_the_protocols() {
        let v = server_version();
        assert_eq!(v.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(v.sdk_protocol, SUPPORTED_PROTOCOL_VERSION);
        assert_eq!(v.mcp_protocol, MCP_PROTOCOL_VERSION);
    }
}
