//! Confirmaciones explícitas del usuario.
//!
//! Cuando la política exige confirmación, la capa de orquestación crea una
//! solicitud pendiente, notifica a la interfaz y espera la decisión del
//! usuario (o el vencimiento del plazo). Los argumentos aprobados se quedan en
//! esta capa: la interfaz solo envía «sí» o «no» para un identificador, de modo
//! que no puede alterarse la operación entre la confirmación y la ejecución.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

use crate::error::{ErrorCode, JarvisError, Result};
use crate::policy::RiskLevel;

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub call_id: String,
    pub tool: String,
    pub summary: String,
    pub preview: Option<String>,
    pub risk: RiskLevel,
    pub origin: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approved,
    Denied,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalEvent {
    Requested {
        request: ApprovalRequest,
    },
    Resolved {
        id: String,
        outcome: ApprovalOutcome,
    },
}

pub type Notifier = Arc<dyn Fn(ApprovalEvent) + Send + Sync>;

type Pending = HashMap<String, (ApprovalRequest, oneshot::Sender<bool>)>;

#[derive(Default)]
pub struct ApprovalBroker {
    pending: Mutex<Pending>,
    notifier: Mutex<Option<Notifier>>,
}

impl ApprovalBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_notifier(&self, notifier: Notifier) {
        *self.notifier.lock().expect("mutex") = Some(notifier);
    }

    fn notify(&self, event: ApprovalEvent) {
        let n = self.notifier.lock().expect("mutex").clone();
        if let Some(n) = n {
            n(event);
        }
    }

    /// Crea una solicitud y espera la decisión del usuario.
    pub async fn request(
        &self,
        mut request: ApprovalRequest,
        timeout: Duration,
    ) -> ApprovalOutcome {
        let (tx, rx) = oneshot::channel();
        request.id = uuid::Uuid::new_v4().to_string();
        request.expires_at_ms = chrono::Utc::now().timestamp_millis() + timeout.as_millis() as i64;
        let id = request.id.clone();
        self.pending
            .lock()
            .expect("mutex")
            .insert(id.clone(), (request.clone(), tx));
        self.notify(ApprovalEvent::Requested { request });

        let outcome = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(true)) => ApprovalOutcome::Approved,
            Ok(Ok(false)) | Ok(Err(_)) => ApprovalOutcome::Denied,
            Err(_) => ApprovalOutcome::TimedOut,
        };
        self.pending.lock().expect("mutex").remove(&id);
        self.notify(ApprovalEvent::Resolved { id, outcome });
        outcome
    }

    /// Registra la decisión del usuario. Cada solicitud solo se resuelve una vez.
    pub fn decide(&self, id: &str, approved: bool) -> Result<()> {
        let entry = self.pending.lock().expect("mutex").remove(id);
        match entry {
            Some((_, tx)) => {
                let _ = tx.send(approved);
                Ok(())
            }
            None => Err(JarvisError::new(
                ErrorCode::NotFound,
                "La solicitud de confirmación ya no está pendiente.",
            )),
        }
    }

    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.pending
            .lock()
            .expect("mutex")
            .values()
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// Rechaza todas las solicitudes pendientes (p. ej. al cerrar la sesión).
    pub fn deny_all(&self) {
        let drained: Vec<_> = self.pending.lock().expect("mutex").drain().collect();
        for (_, (_, tx)) in drained {
            let _ = tx.send(false);
        }
    }
}
