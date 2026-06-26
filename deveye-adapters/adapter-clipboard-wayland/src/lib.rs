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

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use deveye_ports::{Clipboard, PortError, PortResult};

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
        let output = tokio::process::Command::new("wl-paste")
            .arg("--no-newline")
            .output()
            .await
            .map_err(|e| {
                PortError::Unsupported(format!(
                    "wl-paste indisponible (wl-clipboard installé ? session Wayland ?) : {e}"
                ))
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            // Presse-papiers vide : wl-paste sort en code non nul (« Nothing is
            // copied »). Ce n'est pas une erreur — on renvoie une chaîne vide.
            Ok(String::new())
        }
    }

    async fn set(&self, text: &str) -> PortResult<()> {
        // wl-copy lit stdin puis fork un démon qui sert la sélection ; le
        // process de premier plan qu'on lance, lui, se termine vite (on peut
        // donc l'attendre sans bloquer).
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
        {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                PortError::Interaction("wl-copy : stdin indisponible".to_string())
            })?;
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| PortError::Interaction(format!("wl-copy : écriture stdin : {e}")))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| PortError::Interaction(format!("wl-copy : fermeture stdin : {e}")))?;
        }
        let status = child
            .wait()
            .await
            .map_err(|e| PortError::Interaction(format!("wl-copy : attente : {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(PortError::Interaction(format!(
                "wl-copy a échoué (code {:?})",
                status.code()
            )))
        }
    }
}
