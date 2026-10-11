//! Ejecutor autorizado de herramientas.
//!
//! Único punto por el que cualquier interfaz (voz, texto, pruebas) ejecuta una
//! herramienta. Para cada llamada:
//!
//! 1. Idempotencia por `call_id`: si la misma llamada llega dos veces (p. ej.
//!    tras una reconexión), se devuelve el resultado ya obtenido sin repetir la
//!    operación. Llamadas concurrentes con el mismo id esperan a la primera.
//! 2. Validación de nombre, argumentos y rutas.
//! 3. Política de riesgo: ejecutar, pedir confirmación o denegar.
//! 4. Ejecución con límite de tiempo (las escrituras se completan siempre para
//!    no informar de un fallo cuando el fichero sí se ha escrito).
//! 5. Resultado estructurado y registro del estado real en la auditoría.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use crate::anthropic::ClaudeClient;
use crate::approvals::{ApprovalBroker, ApprovalOutcome, ApprovalRequest};
use crate::audit::{AuditEntry, AuditLog};
use crate::error::{ErrorCode, JarvisError, Result};
use crate::policy::{Decision, Policy};
use crate::tools::{self, ToolArgs};
use crate::vault::Vault;

const IDEMPOTENCY_CAPACITY: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Voice,
    Text,
    Test,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Voice => "voice",
            Origin::Text => "text",
            Origin::Test => "test",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    /// Ejecutada y verificada.
    Ok,
    /// Fallo de validación, permisos o ejecución.
    Error,
    /// El usuario rechazó la confirmación o expiró el plazo.
    Denied,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolOutcome {
    pub call_id: String,
    pub tool: String,
    pub status: OutcomeStatus,
    pub data: Option<Value>,
    pub error: Option<JarvisError>,
    /// Solo `true` si la operación se comprobó tras ejecutarse.
    pub verified: bool,
    pub duration_ms: u64,
    /// `true` si se devolvió un resultado previo para el mismo `call_id`.
    pub deduplicated: bool,
}

impl ToolOutcome {
    /// Representación compacta para devolver al modelo.
    pub fn for_model(&self) -> Value {
        match self.status {
            OutcomeStatus::Ok => {
                json!({"ok": true, "verified": self.verified, "result": self.data})
            }
            OutcomeStatus::Denied => json!({
                "ok": false,
                "denied": true,
                "error": self.error.as_ref().map(|e| json!({"code": e.code, "message": e.message})),
                "instruction": "La operación NO se ha realizado. Infórmalo al usuario sin afirmar lo contrario."
            }),
            OutcomeStatus::Error => json!({
                "ok": false,
                "error": self.error.as_ref().map(|e| json!({"code": e.code, "message": e.message, "retryable": e.retryable})),
                "instruction": "La operación ha fallado. No digas que se ha completado."
            }),
        }
    }
}

pub type Clock = Arc<dyn Fn() -> DateTime<Local> + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub tool: Duration,
    pub delegate: Duration,
    pub approval: Duration,
}

pub struct Executor {
    vault: Option<Arc<Vault>>,
    vault_error: Option<JarvisError>,
    claude: Option<Arc<ClaudeClient>>,
    policy: Policy,
    timeouts: Timeouts,
    approvals: Arc<ApprovalBroker>,
    audit: Arc<AuditLog>,
    clock: Clock,
    cache: Mutex<IdempotencyCache>,
}

#[derive(Default)]
struct IdempotencyCache {
    cells: HashMap<String, Arc<OnceCell<ToolOutcome>>>,
    order: VecDeque<String>,
}

impl IdempotencyCache {
    fn cell(&mut self, call_id: &str) -> Arc<OnceCell<ToolOutcome>> {
        if let Some(c) = self.cells.get(call_id) {
            return c.clone();
        }
        let c = Arc::new(OnceCell::new());
        self.cells.insert(call_id.to_string(), c.clone());
        self.order.push_back(call_id.to_string());
        while self.order.len() > IDEMPOTENCY_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.cells.remove(&old);
            }
        }
        c
    }
}

pub struct ExecutorParts {
    pub vault: std::result::Result<Arc<Vault>, JarvisError>,
    pub claude: Option<Arc<ClaudeClient>>,
    pub policy: Policy,
    pub timeouts: Timeouts,
    pub approvals: Arc<ApprovalBroker>,
    pub audit: Arc<AuditLog>,
    pub clock: Clock,
}

impl Executor {
    pub fn new(p: ExecutorParts) -> Self {
        let (vault, vault_error) = match p.vault {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            vault,
            vault_error,
            claude: p.claude,
            policy: p.policy,
            timeouts: p.timeouts,
            approvals: p.approvals,
            audit: p.audit,
            clock: p.clock,
            cache: Mutex::new(IdempotencyCache::default()),
        }
    }

    pub fn has_claude(&self) -> bool {
        self.claude.is_some()
    }

    pub fn vault_status(&self) -> std::result::Result<String, JarvisError> {
        match (&self.vault, &self.vault_error) {
            (Some(v), _) => Ok(v.root().display().to_string()),
            (None, Some(e)) => Err(e.clone()),
            (None, None) => Err(JarvisError::new(
                ErrorCode::VaultNotConfigured,
                "Vault no configurada.",
            )),
        }
    }

    pub fn catalog(&self) -> Vec<tools::ToolSpec> {
        tools::catalog(self.has_claude())
    }

    pub async fn invoke(&self, call: ToolCall) -> ToolOutcome {
        let call_id = call.call_id.trim().to_string();
        if call_id.is_empty() || call_id.len() > 128 {
            let start = Instant::now();
            let outcome = self.finish(
                &call,
                start,
                Err(JarvisError::invalid("call_id ausente o demasiado largo.")),
                None,
            );
            return outcome;
        }
        let cell = self.cache.lock().expect("mutex").cell(&call_id);
        let ran = AtomicBool::new(false);
        let outcome = cell
            .get_or_init(|| async {
                ran.store(true, Ordering::SeqCst);
                self.run(&call).await
            })
            .await
            .clone();
        if ran.load(Ordering::SeqCst) {
            outcome
        } else {
            let mut dup = outcome;
            dup.deduplicated = true;
            self.audit.record(AuditEntry {
                ts: (self.clock)().to_rfc3339(),
                call_id: dup.call_id.clone(),
                tool: dup.tool.clone(),
                origin: call.origin.as_str().into(),
                status: "deduplicated".into(),
                code: None,
                duration_ms: 0,
                target: None,
                deduplicated: true,
            });
            dup
        }
    }

    async fn run(&self, call: &ToolCall) -> ToolOutcome {
        let start = Instant::now();
        let args = match tools::parse_args(&call.name, &call.arguments, self.has_claude()) {
            Ok(a) => a,
            Err(e) => return self.finish(call, start, Err(e), None),
        };
        let risk = tools::spec(&call.name, self.has_claude())
            .map(|s| s.risk)
            .expect("validada");

        match self.policy.decide(risk) {
            Decision::Allow => {}
            Decision::Deny => {
                return self.finish(
                    call,
                    start,
                    Err(JarvisError::new(
                        ErrorCode::PermissionDenied,
                        "La política de permisos de JARVIS no permite esta operación.",
                    )),
                    None,
                )
            }
            Decision::Confirm => {
                let req = ApprovalRequest {
                    id: String::new(),
                    call_id: call.call_id.clone(),
                    tool: call.name.clone(),
                    summary: args.summary(),
                    preview: args.preview(),
                    risk,
                    origin: call.origin.as_str().into(),
                    expires_at_ms: 0,
                };
                match self.approvals.request(req, self.timeouts.approval).await {
                    ApprovalOutcome::Approved => {}
                    ApprovalOutcome::Denied => {
                        return self.finish_denied(
                            call,
                            start,
                            ErrorCode::ApprovalDenied,
                            "El usuario no ha autorizado la operación.",
                        )
                    }
                    ApprovalOutcome::TimedOut => {
                        return self.finish_denied(
                            call,
                            start,
                            ErrorCode::ApprovalTimeout,
                            "No se recibió confirmación a tiempo; la operación no se ha realizado.",
                        )
                    }
                }
            }
        }

        let target = match &args {
            ToolArgs::CreateNote { .. } | ToolArgs::AddTask { .. } => Some(()),
            _ => None,
        };
        let result = self.execute(args).await;
        let target_path = target.and_then(|_| {
            result
                .as_ref()
                .ok()
                .and_then(|v| v.get("path").and_then(Value::as_str).map(str::to_string))
        });
        self.finish(call, start, result, target_path)
    }

    async fn execute(&self, args: ToolArgs) -> Result<Value> {
        let now = (self.clock)();
        let today = now.date_naive();
        match args {
            ToolArgs::DelegateToClaude {
                task,
                context,
                note_paths,
            } => {
                let claude = self.claude.clone().ok_or_else(|| {
                    JarvisError::new(ErrorCode::ToolUnavailable, "Claude no está configurado.")
                })?;
                let mut fragments = Vec::new();
                if !note_paths.is_empty() {
                    let vault = self.vault()?;
                    for p in note_paths {
                        let n = vault.read(&p, Some(&task), 1500)?;
                        fragments.push((n.path, n.content));
                    }
                }
                let fut =
                    async move { claude.delegate(&task, context.as_deref(), &fragments).await };
                let answer = tokio::time::timeout(self.timeouts.delegate, fut)
                    .await
                    .map_err(|_| {
                        JarvisError::new(ErrorCode::Timeout, "Claude tardó demasiado en responder.")
                    })??;
                let (answer, truncated) = crate::vault::truncate_chars(&answer, 6000);
                Ok(json!({"answer": answer, "truncated": truncated, "provider": "claude"}))
            }
            write @ (ToolArgs::CreateNote { .. } | ToolArgs::AddTask { .. }) => {
                // Las escrituras se esperan hasta el final: un timeout aquí
                // podría ocultar una escritura que sí se ha producido.
                let vault = self.vault()?;
                tokio::task::spawn_blocking(move || -> Result<Value> {
                    let r = match write {
                        ToolArgs::CreateNote {
                            title,
                            content,
                            folder,
                        } => vault.create_note(&title, &content, folder.as_deref(), now)?,
                        ToolArgs::AddTask { text, due } => vault.add_task(&text, due)?,
                        _ => unreachable!(),
                    };
                    Ok(serde_json::to_value(r).expect("serializable"))
                })
                .await
                .map_err(|_| {
                    JarvisError::internal("La tarea de escritura terminó inesperadamente.")
                })?
            }
            read => {
                let vault = self.vault()?;
                let job = tokio::task::spawn_blocking(move || -> Result<Value> {
                    Ok(match read {
                        ToolArgs::SearchNotes { query, limit } => {
                            json!({"results": vault.search(&query, limit)?})
                        }
                        ToolArgs::ReadNote {
                            path,
                            query,
                            max_chars,
                        } => {
                            serde_json::to_value(vault.read(&path, query.as_deref(), max_chars)?)
                                .expect("serializable")
                        }
                        ToolArgs::ListTasks(f) => {
                            json!({"tasks": vault.tasks(&f.to_filter(), today)?})
                        }
                        ToolArgs::ListProjects { limit } => {
                            json!({"projects": vault.projects(limit)?})
                        }
                        ToolArgs::DailySummary { date } => {
                            serde_json::to_value(vault.daily_summary(date.unwrap_or(today))?)
                                .expect("serializable")
                        }
                        _ => unreachable!(),
                    })
                });
                tokio::time::timeout(self.timeouts.tool, job)
                    .await
                    .map_err(|_| {
                        JarvisError::new(
                            ErrorCode::Timeout,
                            "La consulta a la Vault tardó demasiado.",
                        )
                    })?
                    .map_err(|_| JarvisError::internal("La consulta terminó inesperadamente."))?
            }
        }
    }

    fn vault(&self) -> Result<Arc<Vault>> {
        self.vault.clone().ok_or_else(|| {
            self.vault_error.clone().unwrap_or_else(|| {
                JarvisError::new(ErrorCode::VaultNotConfigured, "No hay Vault configurada.")
            })
        })
    }

    fn finish_denied(
        &self,
        call: &ToolCall,
        start: Instant,
        code: ErrorCode,
        msg: &str,
    ) -> ToolOutcome {
        let outcome = ToolOutcome {
            call_id: call.call_id.clone(),
            tool: call.name.clone(),
            status: OutcomeStatus::Denied,
            data: None,
            error: Some(JarvisError::new(code, msg)),
            verified: false,
            duration_ms: start.elapsed().as_millis() as u64,
            deduplicated: false,
        };
        self.log(call, &outcome, None);
        outcome
    }

    fn finish(
        &self,
        call: &ToolCall,
        start: Instant,
        result: Result<Value>,
        target: Option<String>,
    ) -> ToolOutcome {
        let outcome = match result {
            Ok(data) => {
                // Las escrituras informan `verified`; las lecturas son su propia verificación.
                let verified = data
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                ToolOutcome {
                    call_id: call.call_id.clone(),
                    tool: call.name.clone(),
                    status: if verified {
                        OutcomeStatus::Ok
                    } else {
                        OutcomeStatus::Error
                    },
                    data: Some(data),
                    error: (!verified).then(|| {
                        JarvisError::new(
                            ErrorCode::VerificationFailed,
                            "No se pudo verificar la operación.",
                        )
                    }),
                    verified,
                    duration_ms: start.elapsed().as_millis() as u64,
                    deduplicated: false,
                }
            }
            Err(e) => ToolOutcome {
                call_id: call.call_id.clone(),
                tool: call.name.clone(),
                status: OutcomeStatus::Error,
                data: None,
                error: Some(e),
                verified: false,
                duration_ms: start.elapsed().as_millis() as u64,
                deduplicated: false,
            },
        };
        self.log(call, &outcome, target);
        outcome
    }

    fn log(&self, call: &ToolCall, o: &ToolOutcome, target: Option<String>) {
        self.audit.record(AuditEntry {
            ts: (self.clock)().to_rfc3339(),
            call_id: o.call_id.clone(),
            tool: o.tool.chars().take(60).collect(),
            origin: call.origin.as_str().into(),
            status: match o.status {
                OutcomeStatus::Ok => "ok",
                OutcomeStatus::Error => "error",
                OutcomeStatus::Denied => "denied",
            }
            .into(),
            code: o.error.as_ref().map(|e| e.code),
            duration_ms: o.duration_ms,
            target,
            deduplicated: false,
        });
    }
}
