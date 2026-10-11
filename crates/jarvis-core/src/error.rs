//! Errores tipados de JARVIS.
//!
//! Cada error lleva un `code` estable (lo consume la interfaz para mostrar
//! mensajes y decidir si reintentar) y un mensaje en español apto para el
//! usuario. Los mensajes nunca incluyen credenciales ni contenido personal.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    MissingApiKey,
    InvalidApiKey,
    RateLimited,
    QuotaExceeded,
    ModelUnavailable,
    ProviderError,
    NetworkError,
    Timeout,
    InvalidArguments,
    UnknownTool,
    ToolUnavailable,
    PermissionDenied,
    ApprovalDenied,
    ApprovalTimeout,
    VaultNotConfigured,
    VaultIo,
    NotFound,
    AlreadyExists,
    VerificationFailed,
    SecretStore,
    Config,
    Internal,
}

impl ErrorCode {
    /// Indica si tiene sentido reintentar la misma operación más tarde.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::RateLimited
                | ErrorCode::ProviderError
                | ErrorCode::NetworkError
                | ErrorCode::Timeout
        )
    }
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct JarvisError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl JarvisError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code.retryable(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArguments, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

pub type Result<T, E = JarvisError> = std::result::Result<T, E>;
