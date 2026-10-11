//! Configuración no secreta de JARVIS.
//!
//! Fuentes, de mayor a menor prioridad:
//! 1. Variables de entorno (`OPENAI_REALTIME_MODEL=...`).
//! 2. Fichero `config.toml` en el directorio de configuración de la app, con
//!    las mismas claves (`OPENAI_REALTIME_MODEL = "..."`).
//! 3. Valores predeterminados documentados en `docs/configuration.md`.
//!
//! Las claves API **no** se leen de aquí: ver [`crate::secrets`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{ErrorCode, JarvisError, Result};
use crate::policy::{Decision, Policy};

pub const DEFAULT_REALTIME_MODEL: &str = "gpt-realtime-2.1";
/// Voz masculina y grave, la más cercana al estilo de JARVIS sin imitar a nadie.
pub const DEFAULT_REALTIME_VOICE: &str = "cedar";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "gpt-4o-mini-transcribe";
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5-5";

/// Voces oficiales de salida de Realtime. `marin` y `cedar` son las
/// recomendadas por OpenAI para máxima naturalidad; todas son multilingües.
pub const REALTIME_VOICES: &[&str] = &[
    "cedar", "marin", "alloy", "ash", "ballad", "coral", "echo", "sage", "shimmer", "verse",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDetection {
    SemanticVad,
    ServerVad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseReduction {
    NearField,
    FarField,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextProvider {
    Auto,
    Claude,
    Commands,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeConfig {
    pub model: String,
    pub voice: String,
    /// Modelo de transcripción de la entrada; `None` desactiva la transcripción.
    pub transcription_model: Option<String>,
    pub language: String,
    pub turn_detection: TurnDetection,
    /// `auto`, `low`, `medium` o `high` (solo `semantic_vad`).
    pub vad_eagerness: String,
    /// Umbral de `server_vad` (0.0–1.0). Más alto = menos sensible al eco.
    pub vad_threshold: f32,
    pub vad_silence_ms: u32,
    pub noise_reduction: NoiseReduction,
    pub secret_ttl_seconds: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultConfig {
    pub path: Option<PathBuf>,
    pub notes_folder: String,
    pub tasks_file: String,
    pub daily_folder: String,
    pub daily_format: String,
    pub projects_folder: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub realtime: RealtimeConfig,
    pub vault: VaultConfig,
    pub policy: Policy,
    pub claude_model: String,
    pub text_provider: TextProvider,
    pub tool_timeout_ms: u64,
    pub delegate_timeout_ms: u64,
    pub approval_timeout_ms: u64,
    pub save_transcripts: bool,
    pub transcript_retention_days: u32,
    pub openai_base_url: String,
    pub anthropic_base_url: String,
}

impl Config {
    /// Carga la configuración desde el entorno y, si existe, `config_file`.
    pub fn load(config_file: Option<&Path>) -> Result<Self> {
        let file = match config_file {
            Some(p) if p.exists() => read_toml(p)?,
            _ => HashMap::new(),
        };
        Self::from_lookup(|key| std::env::var(key).ok().or_else(|| file.get(key).cloned()))
    }

    /// Construye la configuración a partir de una función de búsqueda de claves.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        let voice = get("OPENAI_REALTIME_VOICE").unwrap_or_else(|| DEFAULT_REALTIME_VOICE.into());
        validate_voice(&voice)?;

        let transcription_model = match get("OPENAI_REALTIME_TRANSCRIPTION_MODEL") {
            Some(v) if matches!(v.as_str(), "off" | "none" | "false") => None,
            Some(v) => Some(v),
            None => Some(DEFAULT_TRANSCRIPTION_MODEL.into()),
        };

        let turn_detection = match get("OPENAI_REALTIME_VAD").as_deref() {
            None | Some("semantic_vad") => TurnDetection::SemanticVad,
            Some("server_vad") => TurnDetection::ServerVad,
            Some(other) => return Err(config_err("OPENAI_REALTIME_VAD", other)),
        };

        let vad_eagerness = get("OPENAI_REALTIME_VAD_EAGERNESS").unwrap_or_else(|| "auto".into());
        if !matches!(vad_eagerness.as_str(), "auto" | "low" | "medium" | "high") {
            return Err(config_err("OPENAI_REALTIME_VAD_EAGERNESS", &vad_eagerness));
        }

        let noise_reduction = match get("OPENAI_REALTIME_NOISE_REDUCTION").as_deref() {
            None | Some("far_field") => NoiseReduction::FarField,
            Some("near_field") => NoiseReduction::NearField,
            Some("off") => NoiseReduction::Off,
            Some(other) => return Err(config_err("OPENAI_REALTIME_NOISE_REDUCTION", other)),
        };

        let text_provider = match get("JARVIS_TEXT_PROVIDER").as_deref() {
            None | Some("auto") => TextProvider::Auto,
            Some("claude") => TextProvider::Claude,
            Some("commands") => TextProvider::Commands,
            Some(other) => return Err(config_err("JARVIS_TEXT_PROVIDER", other)),
        };

        let policy = Policy {
            write: parse_decision(
                get("JARVIS_POLICY_WRITE"),
                Decision::Confirm,
                "JARVIS_POLICY_WRITE",
            )?,
            delegate: parse_decision(
                get("JARVIS_POLICY_DELEGATE"),
                Decision::Allow,
                "JARVIS_POLICY_DELEGATE",
            )?,
        };

        Ok(Config {
            realtime: RealtimeConfig {
                model: get("OPENAI_REALTIME_MODEL")
                    .unwrap_or_else(|| DEFAULT_REALTIME_MODEL.into()),
                voice,
                transcription_model,
                language: get("JARVIS_LANGUAGE").unwrap_or_else(|| "es".into()),
                turn_detection,
                vad_eagerness,
                vad_threshold: parse_num(
                    get("OPENAI_REALTIME_VAD_THRESHOLD"),
                    0.6f32,
                    "OPENAI_REALTIME_VAD_THRESHOLD",
                )?
                .clamp(0.0, 1.0),
                vad_silence_ms: parse_num(
                    get("OPENAI_REALTIME_VAD_SILENCE_MS"),
                    600,
                    "OPENAI_REALTIME_VAD_SILENCE_MS",
                )?,
                noise_reduction,
                secret_ttl_seconds: parse_num(
                    get("OPENAI_REALTIME_SECRET_TTL_SECONDS"),
                    600,
                    "OPENAI_REALTIME_SECRET_TTL_SECONDS",
                )?
                .clamp(10, 7200),
            },
            vault: VaultConfig {
                path: get("JARVIS_VAULT_PATH").map(PathBuf::from),
                notes_folder: get("JARVIS_NOTES_FOLDER").unwrap_or_else(|| "JARVIS".into()),
                tasks_file: get("JARVIS_TASKS_FILE").unwrap_or_else(|| "JARVIS/Tareas.md".into()),
                daily_folder: get("JARVIS_DAILY_FOLDER").unwrap_or_else(|| "Diario".into()),
                daily_format: get("JARVIS_DAILY_FORMAT").unwrap_or_else(|| "%Y-%m-%d".into()),
                projects_folder: get("JARVIS_PROJECTS_FOLDER")
                    .unwrap_or_else(|| "Proyectos".into()),
            },
            policy,
            claude_model: get("ANTHROPIC_MODEL").unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.into()),
            text_provider,
            tool_timeout_ms: parse_num(
                get("JARVIS_TOOL_TIMEOUT_MS"),
                15_000,
                "JARVIS_TOOL_TIMEOUT_MS",
            )?,
            delegate_timeout_ms: parse_num(
                get("JARVIS_DELEGATE_TIMEOUT_MS"),
                120_000,
                "JARVIS_DELEGATE_TIMEOUT_MS",
            )?,
            approval_timeout_ms: parse_num(
                get("JARVIS_APPROVAL_TIMEOUT_MS"),
                120_000,
                "JARVIS_APPROVAL_TIMEOUT_MS",
            )?,
            save_transcripts: parse_bool(get("JARVIS_SAVE_TRANSCRIPTS"), false),
            transcript_retention_days: parse_num(
                get("JARVIS_TRANSCRIPT_RETENTION_DAYS"),
                7,
                "JARVIS_TRANSCRIPT_RETENTION_DAYS",
            )?,
            openai_base_url: get("JARVIS_OPENAI_BASE_URL")
                .unwrap_or_else(|| "https://api.openai.com/v1".into()),
            anthropic_base_url: get("JARVIS_ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com/v1".into()),
        })
    }
}

pub fn validate_voice(voice: &str) -> Result<()> {
    if REALTIME_VOICES.contains(&voice) {
        Ok(())
    } else {
        Err(JarvisError::new(
            ErrorCode::Config,
            format!(
                "Voz «{voice}» no reconocida. Voces disponibles: {}",
                REALTIME_VOICES.join(", ")
            ),
        ))
    }
}

fn read_toml(path: &Path) -> Result<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        JarvisError::new(
            ErrorCode::Config,
            format!("No se pudo leer {}: {e}", path.display()),
        )
    })?;
    let table: toml::Table = raw
        .parse()
        .map_err(|e| JarvisError::new(ErrorCode::Config, format!("config.toml inválido: {e}")))?;
    let mut out = HashMap::new();
    for (k, v) in table {
        if k.ends_with("_API_KEY") {
            return Err(JarvisError::new(
                ErrorCode::Config,
                "config.toml no debe contener claves API. Usa el almacén de credenciales (`jarvis secrets set`).",
            ));
        }
        let s = match v {
            toml::Value::String(s) => s,
            other => other.to_string(),
        };
        out.insert(k, s);
    }
    Ok(out)
}

fn config_err(key: &str, value: &str) -> JarvisError {
    JarvisError::new(
        ErrorCode::Config,
        format!("Valor no válido para {key}: «{value}»"),
    )
}

fn parse_num<T: std::str::FromStr>(v: Option<String>, default: T, key: &str) -> Result<T> {
    match v {
        None => Ok(default),
        Some(s) => s.parse().map_err(|_| config_err(key, &s)),
    }
}

fn parse_bool(v: Option<String>, default: bool) -> bool {
    match v.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("1" | "true" | "yes" | "si" | "sí" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

fn parse_decision(v: Option<String>, default: Decision, key: &str) -> Result<Decision> {
    match v.as_deref() {
        None => Ok(default),
        Some("allow") => Ok(Decision::Allow),
        Some("confirm") => Ok(Decision::Confirm),
        Some("deny") => Ok(Decision::Deny),
        Some(other) => Err(config_err(key, other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    #[test]
    fn defaults_are_documented_values() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.realtime.model, DEFAULT_REALTIME_MODEL);
        assert_eq!(c.realtime.voice, "cedar");
        assert_eq!(c.policy.write, Decision::Confirm);
        assert!(!c.save_transcripts);
        assert_eq!(c.realtime.turn_detection, TurnDetection::SemanticVad);
    }

    #[test]
    fn overrides_and_validation() {
        let c = cfg(&[
            ("OPENAI_REALTIME_MODEL", "gpt-realtime-2.1-mini"),
            ("OPENAI_REALTIME_VOICE", "cedar"),
            ("OPENAI_REALTIME_TRANSCRIPTION_MODEL", "off"),
            ("JARVIS_POLICY_WRITE", "allow"),
        ])
        .unwrap();
        assert_eq!(c.realtime.model, "gpt-realtime-2.1-mini");
        assert_eq!(c.realtime.voice, "cedar");
        assert!(c.realtime.transcription_model.is_none());
        assert_eq!(c.policy.write, Decision::Allow);

        assert!(cfg(&[("OPENAI_REALTIME_VOICE", "morgan-freeman")]).is_err());
        assert!(cfg(&[("JARVIS_POLICY_WRITE", "maybe")]).is_err());
    }

    #[test]
    fn toml_rejects_api_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "OPENAI_API_KEY = \"sk-x\"\n").unwrap();
        assert_eq!(read_toml(&p).unwrap_err().code, ErrorCode::Config);
    }
}
