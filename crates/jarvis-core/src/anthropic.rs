//! Cliente mínimo de la API de Claude (Messages API).
//!
//! Claude es un especialista opcional: si no hay clave configurada, JARVIS
//! funciona igual con Realtime y el modo de comandos. No comparte memoria ni
//! historial con Realtime: todo contexto se envía de forma explícita y acotada.

use serde_json::{json, Value};

use crate::error::{ErrorCode, JarvisError, Result};
use crate::http;
use crate::secrets::Secret;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct ClaudeClient {
    http: reqwest::Client,
    base_url: String,
    key: Secret,
    pub model: String,
}

impl ClaudeClient {
    pub fn new(base_url: impl Into<String>, key: Secret, model: impl Into<String>) -> Self {
        Self {
            http: http::client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            key,
            model: model.into(),
        }
    }

    /// Llamada a `POST /messages`. `body` no incluye `model`; se añade aquí.
    pub async fn messages(&self, mut body: Value) -> Result<Value> {
        body["model"] = json!(self.model);
        let resp = self
            .http
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", self.key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| http::network_error("Claude", e))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| http::network_error("Claude", e))?;
        if !(200..300).contains(&status) {
            return Err(http::status_error("Claude", status, &text));
        }
        serde_json::from_str(&text).map_err(|_| {
            JarvisError::new(ErrorCode::ProviderError, "Respuesta inesperada de Claude.")
        })
    }

    /// Delegación de una tarea concreta: un único turno, sin herramientas.
    pub async fn delegate(
        &self,
        task: &str,
        context: Option<&str>,
        fragments: &[(String, String)],
    ) -> Result<String> {
        let mut prompt = format!("<tarea>\n{task}\n</tarea>\n");
        if let Some(c) = context {
            prompt.push_str(&format!("<contexto>\n{c}\n</contexto>\n"));
        }
        for (path, text) in fragments {
            prompt.push_str(&format!("<nota ruta=\"{path}\">\n{text}\n</nota>\n"));
        }
        let body = json!({
            "max_tokens": 4096,
            "system": "Eres un especialista al que JARVIS, un asistente de voz en español, delega tareas de razonamiento extenso, análisis documental o programación. Responde en español, con precisión y de forma estructurada. Empieza con una conclusión de dos o tres frases que pueda leerse en voz alta; después, si hace falta, el detalle. Si la información es insuficiente, dilo explícitamente.",
            "messages": [{"role": "user", "content": prompt}]
        });
        let v = self.messages(body).await?;
        Ok(text_of(&v))
    }
}

/// Concatena los bloques de texto de una respuesta de Messages.
pub fn text_of(v: &Value) -> String {
    v.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
