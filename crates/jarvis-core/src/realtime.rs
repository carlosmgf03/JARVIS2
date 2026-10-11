//! Emisión de secretos temporales para OpenAI Realtime.
//!
//! La clave de larga duración solo existe en este proceso. El cliente (React)
//! recibe un secreto efímero (`ek_…`) que caduca en pocos minutos y solo sirve
//! para abrir una sesión Realtime con la configuración fijada aquí.
//!
//! Endpoint: `POST /v1/realtime/client_secrets` (API Realtime GA).

use std::fmt;

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{validate_voice, NoiseReduction, RealtimeConfig, TurnDetection};
use crate::error::{ErrorCode, JarvisError, Result};
use crate::http;
use crate::secrets::Secret;

/// Configuración de sesión que se comparte con la interfaz para que el SDK
/// envíe exactamente lo mismo en su `session.update` inicial.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSettings {
    pub model: String,
    pub voice: String,
    pub language: String,
    pub transcription_model: Option<String>,
    pub turn_detection: Value,
    pub noise_reduction: Option<String>,
}

/// Credencial efímera para la sesión. Se serializa hacia la interfaz (es su
/// único propósito) pero nunca se registra: `Debug` la oculta.
#[derive(Clone, Serialize)]
pub struct EphemeralSession {
    pub ephemeral_key: String,
    pub expires_at: i64,
    pub settings: SessionSettings,
}

impl fmt::Debug for EphemeralSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EphemeralSession")
            .field("ephemeral_key", &"****")
            .field("expires_at", &self.expires_at)
            .field("settings", &self.settings)
            .finish()
    }
}

pub fn session_settings(cfg: &RealtimeConfig, voice: &str) -> SessionSettings {
    let turn_detection = match cfg.turn_detection {
        TurnDetection::SemanticVad => json!({
            "type": "semantic_vad",
            "eagerness": cfg.vad_eagerness,
            "create_response": true,
            "interrupt_response": true
        }),
        TurnDetection::ServerVad => json!({
            "type": "server_vad",
            "threshold": cfg.vad_threshold,
            "prefix_padding_ms": 300,
            "silence_duration_ms": cfg.vad_silence_ms,
            "create_response": true,
            "interrupt_response": true
        }),
    };
    SessionSettings {
        model: cfg.model.clone(),
        voice: voice.to_string(),
        language: cfg.language.clone(),
        transcription_model: cfg.transcription_model.clone(),
        turn_detection,
        noise_reduction: match cfg.noise_reduction {
            NoiseReduction::NearField => Some("near_field".into()),
            NoiseReduction::FarField => Some("far_field".into()),
            NoiseReduction::Off => None,
        },
    }
}

/// Cuerpo de `POST /realtime/client_secrets`.
pub fn client_secret_body(cfg: &RealtimeConfig, settings: &SessionSettings) -> Value {
    let mut input = json!({
        "format": {"type": "audio/pcm", "rate": 24000},
        "turn_detection": settings.turn_detection,
    });
    if let Some(nr) = &settings.noise_reduction {
        input["noise_reduction"] = json!({"type": nr});
    }
    if let Some(model) = &settings.transcription_model {
        input["transcription"] = json!({"model": model, "language": settings.language});
    }
    json!({
        "expires_after": {"anchor": "created_at", "seconds": cfg.secret_ttl_seconds},
        "session": {
            "type": "realtime",
            "model": settings.model,
            "output_modalities": ["audio"],
            "audio": {
                "input": input,
                "output": {"voice": settings.voice, "format": {"type": "audio/pcm", "rate": 24000}}
            }
        }
    })
}

pub struct RealtimeClient {
    http: reqwest::Client,
    base_url: String,
}

impl RealtimeClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: http::client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn create_session(
        &self,
        key: Option<&Secret>,
        cfg: &RealtimeConfig,
        voice: Option<&str>,
    ) -> Result<EphemeralSession> {
        let key = key.ok_or_else(|| {
            JarvisError::new(
                ErrorCode::MissingApiKey,
                "Falta la clave de OpenAI. Configúrala con `jarvis secrets set openai` o la variable OPENAI_API_KEY.",
            )
        })?;
        let voice = voice.unwrap_or(&cfg.voice);
        validate_voice(voice)?;
        let settings = session_settings(cfg, voice);
        let body = client_secret_body(cfg, &settings);

        let resp = self
            .http
            .post(format!("{}/realtime/client_secrets", self.base_url))
            .bearer_auth(key.expose())
            .json(&body)
            .send()
            .await
            .map_err(|e| http::network_error("OpenAI", e))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| http::network_error("OpenAI", e))?;
        if !(200..300).contains(&status) {
            return Err(http::status_error("OpenAI", status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| {
            JarvisError::new(
                ErrorCode::ProviderError,
                "Respuesta inesperada al crear el secreto temporal.",
            )
        })?;
        let value = v
            .get("value")
            .or_else(|| v.pointer("/client_secret/value"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                JarvisError::new(
                    ErrorCode::ProviderError,
                    "La respuesta no contiene un secreto temporal.",
                )
            })?;
        let expires_at = v
            .get("expires_at")
            .or_else(|| v.pointer("/client_secret/expires_at"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        Ok(EphemeralSession {
            ephemeral_key: value.to_string(),
            expires_at,
            settings,
        })
    }

    /// Comprueba que el modelo existe y está habilitado para la cuenta.
    pub async fn check_model(&self, key: &Secret, model: &str) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/models/{}", self.base_url, model))
            .bearer_auth(key.expose())
            .send()
            .await
            .map_err(|e| http::network_error("OpenAI", e))?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(http::status_error("OpenAI", status, &text))
    }
}
