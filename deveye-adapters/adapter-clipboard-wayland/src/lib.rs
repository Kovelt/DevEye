//! Adapter DevEye : presse-papiers de session **Wayland** via les outils
//! `wl-clipboard` (`wl-copy` / `wl-paste`).
//!
//! Pourquoi un process externe plutôt qu'une crate Wayland : le presse-papiers
//! Wayland exige qu'un client *reste vivant* pour servir la donnée copiée.
//! `wl-copy` gère exactement ça (il fork un démon qui sert la sélection jusqu'à
//! son remplacement). Réimplémenter ce protocole dans DevEye serait disproportionné
//! pour un outil dev-only — on délègue à l'outil de référence, déjà présent sur
//! la machine.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use deveye_ports::{Clipboard, PortError, PortResult};

/// Délai max toléré pour `wl-paste`/`wl-copy` : un daemon wl-copy figé (ou un
/// compositeur muet) tiendrait sinon le gate MCP indéfiniment (audit R1).
const WL_CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(5);

/// Presse-papiers Wayland via `wl-copy` / `wl-paste`.
#[derive(Clone, Debug, Default)]
pub struct WaylandClipboard;

impl WaylandClipboard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Clipboard for WaylandClipboard {
    async fn get(&self) -> PortResult<String> {
        // `--no-newline` : ne pas ajouter de \n final (wl-paste en ajoute un).
        // On garde `child` (spawn plutôt que `.output()`) pour pouvoir le
        // tuer si le timeout expire — `.output()` consomme le process et ne
        // laisse aucune prise dessus en cas de blocage.
        let mut child = tokio::process::Command::new("wl-paste")
            .arg("--no-newline")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PortError::Unsupported(format!(
                    "wl-paste indisponible (wl-clipboard installé ? session Wayland ?) : {e}"
                ))
            })?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| PortError::Unsupported("wl-paste : stdout indisponible".to_string()))?;

        let mut buf = Vec::new();
        let wait = tokio::time::timeout(WL_CLIPBOARD_TIMEOUT, async {
            stdout.read_to_end(&mut buf).await?;
            child.wait().await
        })
        .await;

        match wait {
            Ok(Ok(status)) if status.success() => Ok(String::from_utf8_lossy(&buf).to_string()),
            // Presse-papiers vide : wl-paste sort en code non nul (« Nothing is
            // copied »). Ce n'est pas une erreur — on renvoie une chaîne vide.
            Ok(Ok(_)) => Ok(String::new()),
            Ok(Err(e)) => Err(PortError::Unsupported(format!("wl-paste : attente : {e}"))),
            Err(_) => {
                // start_kill() est non bloquant ; tokio récupère (reap) le
                // process tué via sa file d'orphelins interne, sans wait() de plus.
                let _ = child.start_kill();
                tracing::warn!(
                    timeout_s = WL_CLIPBOARD_TIMEOUT.as_secs(),
                    "wl-paste : délai dépassé, process tué (compositeur muet ?)"
                );
                Err(PortError::Timeout(format!(
                    "wl-paste : délai dépassé ({} s), process tué",
                    WL_CLIPBOARD_TIMEOUT.as_secs()
                )))
            }
        }
    }

    async fn set(&self, text: &str) -> PortResult<()> {
        // wl-copy lit stdin puis fork un démon qui sert la sélection — mais il
        // ne touche à stdin qu'APRÈS sa négociation Wayland (connect +
        // roundtrip). Un compositeur muet bloque donc `write_all` dès que le
        // texte dépasse la capacité du tube (64 Ko) : toute la séquence
        // écriture + fermeture + attente doit tenir dans le MÊME timeout.
        let mut child = tokio::process::Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PortError::Unsupported(format!(
                    "wl-copy indisponible (wl-clipboard installé ? session Wayland ?) : {e}"
                ))
            })?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| PortError::Interaction("wl-copy : stdin indisponible".to_string()))?;
        let io =
            tokio::time::timeout(WL_CLIPBOARD_TIMEOUT, async {
                stdin.write_all(text.as_bytes()).await.map_err(|e| {
                    PortError::Interaction(format!("wl-copy : écriture stdin : {e}"))
                })?;
                stdin.shutdown().await.map_err(|e| {
                    PortError::Interaction(format!("wl-copy : fermeture stdin : {e}"))
                })?;
                // EOF explicite : wl-copy ne se termine qu'une fois stdin fermé.
                drop(stdin);
                child
                    .wait()
                    .await
                    .map_err(|e| PortError::Interaction(format!("wl-copy : attente : {e}")))
            })
            .await;
        match io {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => Err(PortError::Interaction(format!(
                "wl-copy a échoué (code {:?})",
                status.code()
            ))),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // start_kill() est non bloquant ; tokio récupère (reap) le
                // process tué via sa file d'orphelins interne, sans wait() de plus.
                let _ = child.start_kill();
                tracing::warn!(
                    timeout_s = WL_CLIPBOARD_TIMEOUT.as_secs(),
                    "wl-copy : délai dépassé, process tué (compositeur muet ?)"
                );
                Err(PortError::Timeout(format!(
                    "wl-copy : délai dépassé ({} s), process tué",
                    WL_CLIPBOARD_TIMEOUT.as_secs()
                )))
            }
        }
    }
}
