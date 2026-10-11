//! Política de permisos.
//!
//! Cada herramienta declara un nivel de riesgo. La política decide si se
//! ejecuta directamente, si requiere confirmación explícita del usuario o si
//! está prohibida. La política es la misma para voz y texto: el origen de la
//! petición no relaja nunca los permisos.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Consultas y lecturas de la Vault. Sin efectos.
    Read,
    /// Crea o añade contenido en la Vault (nunca sobrescribe ni borra).
    Write,
    /// Envía contexto a un proveedor externo (Claude).
    Delegate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Confirm,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Policy {
    pub write: Decision,
    pub delegate: Decision,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            write: Decision::Confirm,
            delegate: Decision::Allow,
        }
    }
}

impl Policy {
    pub fn decide(&self, risk: RiskLevel) -> Decision {
        match risk {
            RiskLevel::Read => Decision::Allow,
            RiskLevel::Write => self.write,
            RiskLevel::Delegate => self.delegate,
        }
    }
}
