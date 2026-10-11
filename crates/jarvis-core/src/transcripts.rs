//! Retención de transcripciones y métricas de uso.
//!
//! Por defecto **no** se guarda nada: las transcripciones viven solo en la
//! memoria de la interfaz y desaparecen al cerrar la sesión. Si el usuario
//! activa `JARVIS_SAVE_TRANSCRIPTS`, se guardan como JSON en el directorio de
//! datos de la aplicación (no en la Vault) y se purgan pasados
//! `JARVIS_TRANSCRIPT_RETENTION_DAYS` días. El audio nunca se guarda.
//!
//! Las métricas de uso registran solo recuentos (tokens, duración), nunca
//! contenido.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, JarvisError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub role: String,
    pub text: String,
    pub ts: String,
}

pub struct TranscriptStore {
    dir: Option<PathBuf>,
    pub enabled: bool,
    pub retention_days: u32,
}

impl TranscriptStore {
    pub fn new(dir: Option<PathBuf>, enabled: bool, retention_days: u32) -> Self {
        Self {
            dir,
            enabled,
            retention_days,
        }
    }

    pub fn save(&self, session_id: &str, entries: &[TranscriptEntry]) -> Result<Option<String>> {
        if !self.enabled || entries.is_empty() {
            return Ok(None);
        }
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| JarvisError::internal("Sin directorio de datos."))?;
        if session_id.is_empty()
            || session_id.len() > 64
            || !session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(JarvisError::invalid("Identificador de sesión no válido."));
        }
        fs::create_dir_all(dir).map_err(io)?;
        let name = format!(
            "{}_{session_id}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let path = dir.join(name);
        let json = serde_json::to_vec_pretty(entries)
            .map_err(|_| JarvisError::internal("serialización"))?;
        fs::write(&path, json).map_err(io)?;
        Ok(Some(path.display().to_string()))
    }

    /// Elimina transcripciones más antiguas que el periodo de retención.
    pub fn purge_expired(&self) -> usize {
        let Some(dir) = &self.dir else { return 0 };
        let max_age = Duration::from_secs(u64::from(self.retention_days) * 86_400);
        let now = SystemTime::now();
        let mut removed = 0;
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let old = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|age| age > max_age);
                if old
                    && e.path().extension().is_some_and(|x| x == "json")
                    && fs::remove_file(e.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        removed
    }

    pub fn clear(&self) -> usize {
        let Some(dir) = &self.dir else { return 0 };
        let mut removed = 0;
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "json")
                    && fs::remove_file(e.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        removed
    }
}

fn io(e: std::io::Error) -> JarvisError {
    JarvisError::new(
        ErrorCode::Internal,
        format!("No se pudo guardar la transcripción: {e}"),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageRecord {
    #[serde(default)]
    pub ts: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsageSummary {
    pub sessions: u64,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub duration_seconds: u64,
}

pub struct UsageLog {
    path: Option<PathBuf>,
}

impl UsageLog {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    pub fn record(&self, mut r: UsageRecord) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        r.ts = chrono::Local::now().to_rfc3339();
        r.model = r.model.chars().take(64).collect();
        if let Some(d) = path.parent() {
            fs::create_dir_all(d).map_err(io)?;
        }
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io)?;
        writeln!(f, "{}", serde_json::to_string(&r).unwrap_or_default()).map_err(io)
    }

    /// Suma de los últimos 30 días.
    pub fn summary(&self) -> UsageSummary {
        let mut s = UsageSummary::default();
        let Some(path) = &self.path else { return s };
        let cutoff = chrono::Local::now() - chrono::Duration::days(30);
        for line in fs::read_to_string(path).unwrap_or_default().lines() {
            let Ok(r) = serde_json::from_str::<UsageRecord>(line) else {
                continue;
            };
            let recent = chrono::DateTime::parse_from_rfc3339(&r.ts)
                .map(|t| t > cutoff)
                .unwrap_or(false);
            if !recent {
                continue;
            }
            s.sessions += 1;
            s.requests += r.requests;
            s.input_tokens += r.input_tokens;
            s.output_tokens += r.output_tokens;
            s.total_tokens += r.total_tokens;
            s.duration_seconds += r.duration_seconds;
        }
        s
    }
}
