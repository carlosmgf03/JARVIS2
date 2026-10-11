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
    /// Consulta Internet (tiempo, búsqueda, leer una página). Sin efectos,
    /// pero envía la consulta o la dirección a un servicio externo.
    Web,
    /// Abre una página en el navegador del usuario (acción externa visible).
    OpenUrl,
    /// Lee el correo del usuario (solo lectura, nunca envía ni borra).
    Email,
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
    pub web: Decision,
    pub open: Decision,
    pub email: Decision,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            write: Decision::Confirm,
            delegate: Decision::Allow,
            web: Decision::Allow,
            open: Decision::Confirm,
            email: Decision::Allow,
        }
    }
}

impl Policy {
    pub fn decide(&self, risk: RiskLevel) -> Decision {
        match risk {
            RiskLevel::Read => Decision::Allow,
            RiskLevel::Write => self.write,
            RiskLevel::Delegate => self.delegate,
            RiskLevel::Web => self.web,
            RiskLevel::OpenUrl => self.open,
            RiskLevel::Email => self.email,
        }
    }
}
