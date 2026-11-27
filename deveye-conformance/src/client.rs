//! Client HTTP minimal de la suite : un seul chemin pour émettre des requêtes
//! arbitraires (Host forcé, corps invalide, méthode inattendue) et un chemin
//! TCP brut pour ce que `reqwest` refuse d'émettre (requête sans `Host`).

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ConformanceError;

/// Délai maximal d'une requête : au-dessus des 10 s du `504` d'évaluation.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Réponse HTTP réduite à ce que la suite inspecte.
pub(crate) struct Resp {
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

impl Resp {
    #[cfg(test)]
    pub(crate) fn for_test(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.to_string(),
        }
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub(crate) fn json(&self) -> Result<Value, String> {
        serde_json::from_str(&self.body)
            .map_err(|e| format!("corps non JSON ({e}) : {}", body_size(&self.body)))
    }
}

/// Taille d'un corps pour un message d'erreur. Jamais son contenu : un corps
/// de réponse peut porter des données de l'app testée (état, logs), et les
/// rapports finissent dans des journaux de CI.
pub(crate) fn body_size(text: &str) -> String {
    format!("{} octets", text.len())
}

/// Cible de la suite : une URL de base et le client qui la joint.
pub(crate) struct Target {
    base: String,
    /// `hôte:port` tels qu'ils figurent dans l'URL (IPv6 entre crochets).
    pub(crate) authority: String,
    pub(crate) port: u16,
    http: reqwest::Client,
}

impl Target {
    pub(crate) fn new(url: &str) -> Result<Self, ConformanceError> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| ConformanceError::InvalidUrl(format!("{url} : {e}")))?;
        if parsed.scheme() != "http" {
            return Err(ConformanceError::InvalidUrl(format!(
                "{url} : seul http:// est supporté (le protocole n'a pas de TLS)"
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| ConformanceError::InvalidUrl(format!("{url} : hôte absent")))?;
        let port = parsed.port_or_known_default().unwrap_or(80);
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            // Une variable HTTP_PROXY ne doit jamais s'intercaler : on teste
            // l'implémentation, pas un proxy.
            .no_proxy()
            .build()
            .map_err(|e| ConformanceError::Internal(e.to_string()))?;
        Ok(Self {
            base: format!("http://{host}:{port}"),
            authority: format!("{host}:{port}"),
            port,
            http,
        })
    }

    /// Requête arbitraire ; `Host` n'est forcé que si `headers` le contient.
    pub(crate) async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<Resp, String> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("méthode invalide : {e}"))?;
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("en-tête {name} invalide : {e}"))?;
            let value = HeaderValue::from_str(value)
                .map_err(|e| format!("valeur d'en-tête invalide : {e}"))?;
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("requête impossible : {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|e| format!("lecture du corps impossible : {e}"))?;
        Ok(Resp {
            status,
            headers,
            body,
        })
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Resp, String> {
        self.send("GET", path, &[], None).await
    }

    pub(crate) async fn post_json(&self, path: &str, body: &Value) -> Result<Resp, String> {
        self.send(
            "POST",
            path,
            &[("content-type", "application/json")],
            Some(body.to_string().into_bytes()),
        )
        .await
    }

    /// Envoie un corps JSON brut (éventuellement invalide) en `POST`.
    pub(crate) async fn post_raw(&self, path: &str, body: &[u8]) -> Result<Resp, String> {
        self.send(
            "POST",
            path,
            &[("content-type", "application/json")],
            Some(body.to_vec()),
        )
        .await
    }

    /// Requête HTTP/1.1 **sans en-tête `Host`**, par TCP brut. Retourne la
    /// ligne de statut et les en-têtes, en minuscules.
    pub(crate) async fn raw_without_host(&self, path: &str) -> Result<(u16, String), String> {
        let work = async {
            let mut stream = tokio::net::TcpStream::connect(&self.authority)
                .await
                .map_err(|e| format!("connexion impossible : {e}"))?;
            let request = format!("GET {path} HTTP/1.1\r\nConnection: close\r\n\r\n");
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| format!("écriture impossible : {e}"))?;
            let mut raw = Vec::new();
            stream
                .read_to_end(&mut raw)
                .await
                .map_err(|e| format!("lecture impossible : {e}"))?;
            Ok::<_, String>(String::from_utf8_lossy(&raw).to_lowercase())
        };
        let text = tokio::time::timeout(REQUEST_TIMEOUT, work)
            .await
            .map_err(|_| "délai dépassé sans réponse".to_string())??;
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| format!("ligne de statut illisible ({})", body_size(&text)))?;
        Ok((status, text))
    }
}
