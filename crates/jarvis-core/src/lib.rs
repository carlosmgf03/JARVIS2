//! Núcleo de JARVIS (capa de orquestación).
//!
//! Este crate no depende de Tauri: lo usan la aplicación de escritorio
//! (`src-tauri`), la CLI `jarvis` y las pruebas. Ver `docs/architecture.md`.

pub mod anthropic;
pub mod approvals;
pub mod audit;
pub mod config;
pub mod email;
pub mod error;
pub mod executor;
pub mod flights;
pub mod http;
pub mod persona;
pub mod places;
pub mod policy;
pub mod realtime;
pub mod secrets;
pub mod text_agent;
pub mod tools;
pub mod traffic;
pub mod transcripts;
pub mod vault;
pub mod web;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::anthropic::ClaudeClient;
use crate::approvals::ApprovalBroker;
use crate::audit::AuditLog;
use crate::config::{Config, REALTIME_VOICES};
use crate::email::EmailClient;
use crate::error::Result;
use crate::executor::{Executor, ExecutorParts, Timeouts};
use crate::policy::Policy;
use crate::realtime::{EphemeralSession, RealtimeClient};
use crate::secrets::{Credentials, Provider, SecretSource};
use crate::text_agent::TextAgent;
use crate::tools::ToolSpec;
use crate::transcripts::{TranscriptStore, UsageLog};
use crate::vault::Vault;
use crate::web::{SystemOpener, UrlOpener, WebClient};

/// Fachada con todos los servicios de JARVIS.
pub struct Jarvis {
    pub config: Config,
    credentials: Arc<Credentials>,
    pub approvals: Arc<ApprovalBroker>,
    pub audit: Arc<AuditLog>,
    pub executor: Arc<Executor>,
    pub text: TextAgent,
    realtime: RealtimeClient,
    pub transcripts: TranscriptStore,
    pub usage: UsageLog,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    pub ok: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub openai_key: SecretSource,
    pub anthropic_key: SecretSource,
    pub vault: VaultStatus,
    pub realtime_model: String,
    pub default_voice: String,
    pub voices: Vec<&'static str>,
    pub transcription_enabled: bool,
    pub text_provider: &'static str,
    pub policy: Policy,
    pub save_transcripts: bool,
    pub transcript_retention_days: u32,
    /// Capacidades opcionales activas.
    pub web_search: bool,
    pub email: Option<String>,
    pub weather_location: Option<String>,
    pub traffic: bool,
    pub traffic_city: String,
    pub flights: bool,
    /// Verdadero si hay credenciales para intentar una sesión de voz real.
    pub voice_available: bool,
    /// Motivos por los que la app está en modo de prueba (vacío = todo listo).
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeSessionResponse {
    pub session: EphemeralSession,
    pub instructions: String,
    pub tools: Vec<ToolSpec>,
}

impl Jarvis {
    /// `data_dir`: directorio privado de la app (auditoría, transcripciones
    /// opcionales, métricas). `None` = todo en memoria (pruebas).
    pub fn new(config: Config, credentials: Credentials, data_dir: Option<PathBuf>) -> Self {
        Self::with_opener(config, credentials, data_dir, Some(Arc::new(SystemOpener)))
    }

    /// Igual que [`Jarvis::new`] pero con un abridor de URLs propio (o ninguno).
    pub fn with_opener(
        config: Config,
        credentials: Credentials,
        data_dir: Option<PathBuf>,
        opener: Option<Arc<dyn UrlOpener>>,
    ) -> Self {
        let credentials = Arc::new(credentials);
        let approvals = Arc::new(ApprovalBroker::new());
        let audit = Arc::new(AuditLog::new(
            data_dir.as_ref().map(|d| d.join("audit.log")),
        ));
        let claude = credentials.get(Provider::Anthropic).map(|k| {
            Arc::new(ClaudeClient::new(
                &config.anthropic_base_url,
                k,
                &config.claude_model,
            ))
        });
        let executor = Arc::new(Executor::new(ExecutorParts {
            vault: Vault::open(&config.vault).map(Arc::new),
            claude: claude.clone(),
            policy: config.policy,
            timeouts: Timeouts {
                tool: Duration::from_millis(config.tool_timeout_ms),
                delegate: Duration::from_millis(config.delegate_timeout_ms),
                approval: Duration::from_millis(config.approval_timeout_ms),
            },
            approvals: approvals.clone(),
            audit: audit.clone(),
            clock: Arc::new(chrono::Local::now),
            web: Arc::new(WebClient::new(config.web.clone(), &config.openai_base_url)),
            email: EmailClient::new(&config.email).map(Arc::new),
            opener,
            secrets: {
                let c = credentials.clone();
                Arc::new(move |p| c.get(p))
            },
            traffic: Arc::new(crate::traffic::TrafficClient::new(config.traffic.clone())),
            flights: Arc::new(crate::flights::FlightsClient::new(config.flights.clone())),
        }));
        let text = TextAgent::new(executor.clone(), claude, config.text_provider);
        let transcripts = TranscriptStore::new(
            data_dir.as_ref().map(|d| d.join("transcripts")),
            config.save_transcripts,
            config.transcript_retention_days,
        );
        transcripts.purge_expired();
        Self {
            realtime: RealtimeClient::new(&config.openai_base_url),
            usage: UsageLog::new(data_dir.as_ref().map(|d| d.join("usage.jsonl"))),
            config,
            credentials,
            approvals,
            audit,
            executor,
            text,
            transcripts,
        }
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    pub fn status(&self) -> Status {
        let openai = self.credentials.source(Provider::OpenAi);
        let anthropic = self.credentials.source(Provider::Anthropic);
        let vault = match self.executor.vault_status() {
            Ok(p) => VaultStatus {
                ok: true,
                path: Some(p),
                error: None,
            },
            Err(e) => VaultStatus {
                ok: false,
                path: None,
                error: Some(e.message),
            },
        };
        let mut limitations = Vec::new();
        if openai == SecretSource::Missing {
            limitations.push(
                "Falta la clave de OpenAI: la conversación de voz real no está disponible.".into(),
            );
        }
        if !vault.ok {
            limitations.push("La Vault de Obsidian no está configurada o no es accesible.".into());
        }
        Status {
            voice_available: openai != SecretSource::Missing,
            openai_key: openai,
            anthropic_key: anthropic,
            vault,
            realtime_model: self.config.realtime.model.clone(),
            default_voice: self.config.realtime.voice.clone(),
            voices: REALTIME_VOICES.to_vec(),
            transcription_enabled: self.config.realtime.transcription_model.is_some(),
            text_provider: self.text.provider(),
            policy: self.config.policy,
            save_transcripts: self.transcripts.enabled,
            transcript_retention_days: self.transcripts.retention_days,
            web_search: self.executor.availability().web_search,
            email: self
                .executor
                .availability()
                .email
                .then(|| self.config.email.address.clone())
                .flatten(),
            weather_location: self.config.web.weather_location.clone(),
            traffic: self.executor.availability().traffic,
            traffic_city: self.config.traffic.city.clone(),
            flights: self.executor.availability().flights,
            limitations,
        }
    }

    /// Emite un secreto temporal y devuelve todo lo que la interfaz necesita
    /// para abrir la sesión Realtime: credencial efímera, configuración de
    /// audio, instrucciones y catálogo de herramientas.
    pub async fn create_realtime_session(
        &self,
        voice: Option<&str>,
        carry_over: Option<&str>,
    ) -> Result<RealtimeSessionResponse> {
        let key = self.credentials.get(Provider::OpenAi);
        let session = self
            .realtime
            .create_session(key.as_ref(), &self.config.realtime, voice)
            .await?;
        Ok(RealtimeSessionResponse {
            session,
            instructions: persona::voice_instructions(chrono::Local::now(), carry_over),
            tools: self.executor.catalog(),
        })
    }

    pub async fn check_realtime_model(&self) -> Result<()> {
        let key = self.credentials.get(Provider::OpenAi).ok_or_else(|| {
            error::JarvisError::new(error::ErrorCode::MissingApiKey, "Falta la clave de OpenAI.")
        })?;
        self.realtime
            .check_model(&key, &self.config.realtime.model)
            .await
    }
}
