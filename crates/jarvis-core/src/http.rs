//! Utilidades HTTP compartidas por los proveedores (OpenAI y Anthropic).

use std::time::Duration;

use serde_json::Value;

use crate::error::{ErrorCode, JarvisError};
use crate::secrets::redact;

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("jarvis/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("cliente HTTP")
}

pub fn network_error(provider: &str, e: reqwest::Error) -> JarvisError {
    if e.is_timeout() {
        JarvisError::new(
            ErrorCode::Timeout,
            format!("{provider} no respondió a tiempo."),
        )
    } else {
        JarvisError::new(
            ErrorCode::NetworkError,
            format!("No se pudo conectar con {provider}. Revisa la conexión a Internet."),
        )
    }
}

/// Traduce una respuesta HTTP de error a un error tipado, sin filtrar claves.
pub fn status_error(provider: &str, status: u16, body: &str) -> JarvisError {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let err = parsed.as_ref().and_then(|v| v.get("error"));
    let code = err
        .and_then(|e| e.get("code").or_else(|| e.get("type")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let detail = redact(
        err.and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let detail: String = detail.chars().take(300).collect();
    let mentions_model = code.contains("model") || detail.to_lowercase().contains("model");

    let (code_out, msg) = match status {
        401 => (
            ErrorCode::InvalidApiKey,
            format!("{provider} rechazó la clave API (401). Revísala."),
        ),
        403 if mentions_model => (
            ErrorCode::ModelUnavailable,
            format!("La cuenta no tiene acceso al modelo solicitado en {provider}."),
        ),
        403 => (
            ErrorCode::PermissionDenied,
            format!("{provider} denegó la operación (403)."),
        ),
        404 => (
            ErrorCode::ModelUnavailable,
            format!("{provider}: modelo o recurso no disponible (404)."),
        ),
        400 | 422 if mentions_model => (
            ErrorCode::ModelUnavailable,
            format!("{provider}: el modelo configurado no es válido o no está habilitado."),
        ),
        429 if code.contains("quota") || detail.to_lowercase().contains("quota") => (
            ErrorCode::QuotaExceeded,
            format!("{provider}: cuota agotada. Revisa la facturación de la cuenta."),
        ),
        429 => (
            ErrorCode::RateLimited,
            format!("{provider}: límite de solicitudes alcanzado. Reintenta en unos segundos."),
        ),
        500..=599 => (
            ErrorCode::ProviderError,
            format!("{provider} tiene un error temporal ({status})."),
        ),
        _ => (
            ErrorCode::ProviderError,
            format!("{provider} devolvió un error ({status})."),
        ),
    };
    let full = if detail.is_empty() {
        msg
    } else {
        format!("{msg} Detalle: {detail}")
    };
    JarvisError::new(code_out, full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_statuses() {
        assert_eq!(
            status_error("OpenAI", 401, "{}").code,
            ErrorCode::InvalidApiKey
        );
        assert_eq!(
            status_error("OpenAI", 429, "{}").code,
            ErrorCode::RateLimited
        );
        assert_eq!(
            status_error(
                "OpenAI",
                429,
                r#"{"error":{"code":"insufficient_quota","message":"x"}}"#
            )
            .code,
            ErrorCode::QuotaExceeded
        );
        assert_eq!(
            status_error(
                "OpenAI",
                400,
                r#"{"error":{"code":"model_not_found","message":"The model does not exist"}}"#
            )
            .code,
            ErrorCode::ModelUnavailable
        );
        assert_eq!(
            status_error("OpenAI", 503, "").code,
            ErrorCode::ProviderError
        );
        assert!(status_error("OpenAI", 503, "").retryable);
    }

    #[test]
    fn never_echoes_keys() {
        let e = status_error(
            "OpenAI",
            401,
            concat!(
                r#"{"error":{"message":"Incorrect API key provided: sk-"#,
                r#"proj-abc123456789xyz"}}"#
            ),
        );
        assert!(!e.message.contains("abc123456789xyz"));
    }
}
