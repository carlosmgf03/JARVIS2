//! Registro de operaciones.
//!
//! Guarda el estado **real** de cada llamada a herramienta (no lo que el modelo
//! dijo): nombre, origen, resultado, código de error, duración y, en las
//! escrituras, la ruta afectada. No guarda argumentos ni contenido de las
//! conversaciones, ni credenciales.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

use crate::error::ErrorCode;

const MAX_RECENT: usize = 200;
const MAX_FILE_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub ts: String,
    pub call_id: String,
    pub tool: String,
    pub origin: String,
    pub status: String,
    pub code: Option<ErrorCode>,
    pub duration_ms: u64,
    pub target: Option<String>,
    pub deduplicated: bool,
}

pub struct AuditLog {
    path: Option<PathBuf>,
    recent: Mutex<VecDeque<AuditEntry>>,
}

impl AuditLog {
    /// `path = None` mantiene el registro solo en memoria.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            recent: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self, entry: AuditEntry) {
        tracing::info!(
            tool = %entry.tool,
            origin = %entry.origin,
            status = %entry.status,
            code = ?entry.code,
            duration_ms = entry.duration_ms,
            "herramienta"
        );
        if let Some(path) = &self.path {
            if let Err(e) = append(path, &entry) {
                tracing::warn!("no se pudo escribir el registro de auditoría: {e}");
            }
        }
        let mut r = self.recent.lock().expect("mutex");
        r.push_back(entry);
        while r.len() > MAX_RECENT {
            r.pop_front();
        }
    }

    pub fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        let r = self.recent.lock().expect("mutex");
        r.iter().rev().take(limit).cloned().collect()
    }
}

fn append(path: &PathBuf, entry: &AuditEntry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path)
        .map(|m| m.len() > MAX_FILE_BYTES)
        .unwrap_or(false)
    {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(entry).unwrap_or_default();
    writeln!(f, "{line}")
}
