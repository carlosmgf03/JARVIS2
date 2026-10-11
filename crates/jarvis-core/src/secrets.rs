//! Gestión de credenciales de larga duración.
//!
//! Las claves se leen de variables de entorno o del almacén de credenciales del
//! sistema operativo (Llavero de macOS, Administrador de credenciales de
//! Windows, Secret Service en Linux). Nunca se envían a la interfaz, nunca se
//! escriben en ficheros del proyecto ni en la Vault y nunca se registran:
//! [`Secret`] oculta su valor en `Debug` y no implementa `Display` ni
//! `Serialize`.

use std::fmt;

use serde::Serialize;

use crate::error::{ErrorCode, JarvisError, Result};

/// Servicio bajo el que se guardan las claves en el almacén del sistema.
pub const KEYCHAIN_SERVICE: &str = "com.jarvis.desktop";

/// Valor secreto que no se puede imprimir ni serializar por accidente.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Acceso explícito al valor. Úsalo solo para construir la cabecera de
    /// autenticación de una petición saliente.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(****)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    OpenAi,
    Anthropic,
    /// Contraseña de aplicación del correo (IMAP).
    Email,
    /// Clave de TomTom (tráfico).
    TomTom,
    /// Clave de SerpApi (vuelos de Google Flights).
    SerpApi,
}

impl Provider {
    pub fn env_var(self) -> &'static str {
        match self {
            Provider::OpenAi => "OPENAI_API_KEY",
            Provider::Anthropic => "ANTHROPIC_API_KEY",
            Provider::Email => "JARVIS_EMAIL_PASSWORD",
            Provider::TomTom => "TOMTOM_API_KEY",
            Provider::SerpApi => "SERPAPI_API_KEY",
        }
    }

    pub fn keychain_user(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Email => "email",
            Provider::TomTom => "tomtom",
            Provider::SerpApi => "serpapi",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "openai" => Some(Provider::OpenAi),
            "anthropic" | "claude" => Some(Provider::Anthropic),
            "email" | "correo" | "mail" => Some(Provider::Email),
            "tomtom" | "trafico" | "tráfico" => Some(Provider::TomTom),
            "serpapi" | "vuelos" => Some(Provider::SerpApi),
            _ => None,
        }
    }
}

/// De dónde procede una clave (para mostrar el estado sin revelar el valor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    Environment,
    Keychain,
    Missing,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, provider: Provider) -> Result<Option<Secret>>;
    fn set(&self, provider: Provider, value: &Secret) -> Result<()>;
    fn delete(&self, provider: Provider) -> Result<()>;
}

/// Almacén del sistema operativo mediante `keyring`.
pub struct KeychainStore;

impl KeychainStore {
    fn entry(provider: Provider) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, provider.keychain_user()).map_err(|e| {
            JarvisError::new(
                ErrorCode::SecretStore,
                format!("No se pudo abrir el almacén de credenciales del sistema: {e}"),
            )
        })
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, provider: Provider) -> Result<Option<Secret>> {
        match Self::entry(provider)?.get_password() {
            Ok(v) if !v.trim().is_empty() => Ok(Some(Secret::new(v.trim()))),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(JarvisError::new(
                ErrorCode::SecretStore,
                format!("No se pudo leer el almacén de credenciales: {e}"),
            )),
        }
    }

    fn set(&self, provider: Provider, value: &Secret) -> Result<()> {
        Self::entry(provider)?
            .set_password(value.expose())
            .map_err(|e| {
                JarvisError::new(
                    ErrorCode::SecretStore,
                    format!("No se pudo guardar la clave en el almacén del sistema: {e}"),
                )
            })
    }

    fn delete(&self, provider: Provider) -> Result<()> {
        match Self::entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(JarvisError::new(
                ErrorCode::SecretStore,
                format!("No se pudo eliminar la clave: {e}"),
            )),
        }
    }
}

/// Búsqueda de variables de entorno (inyectable en pruebas).
pub type EnvLookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Resolución de claves: primero la variable de entorno, después el almacén
/// del sistema. Un fallo del almacén (p. ej. Linux sin Secret Service) se trata
/// como "sin clave" para que la app siga funcionando en modo texto.
pub struct Credentials {
    store: Box<dyn SecretStore>,
    env: EnvLookup,
}

impl Credentials {
    pub fn new(store: Box<dyn SecretStore>, env: EnvLookup) -> Self {
        Self { store, env }
    }

    pub fn system() -> Self {
        Self::new(Box::new(KeychainStore), Box::new(|k| std::env::var(k).ok()))
    }

    pub fn get(&self, provider: Provider) -> Option<Secret> {
        self.resolve(provider).0
    }

    pub fn source(&self, provider: Provider) -> SecretSource {
        self.resolve(provider).1
    }

    fn resolve(&self, provider: Provider) -> (Option<Secret>, SecretSource) {
        if let Some(v) = (self.env)(provider.env_var()) {
            let v = v.trim();
            if !v.is_empty() {
                return (Some(Secret::new(v)), SecretSource::Environment);
            }
        }
        match self.store.get(provider) {
            Ok(Some(s)) => (Some(s), SecretSource::Keychain),
            Ok(None) => (None, SecretSource::Missing),
            Err(e) => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(
                    || tracing::warn!(code = ?e.code, "almacén de credenciales no disponible"),
                );
                (None, SecretSource::Missing)
            }
        }
    }

    pub fn store(&self) -> &dyn SecretStore {
        self.store.as_ref()
    }
}

/// Elimina de un texto cualquier cosa con forma de clave antes de mostrarlo o
/// registrarlo (defensa adicional ante mensajes de error de proveedores).
pub fn redact(text: &str) -> String {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(sk-[A-Za-z0-9_\-]{8,}|ek_[A-Za-z0-9_\-]{8,}|sk-ant-[A-Za-z0-9_\-]{8,})",
        )
        .expect("regex válida")
    });
    re.replace_all(text, "[REDACTADO]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore(Mutex<Option<String>>);

    impl SecretStore for MemStore {
        fn get(&self, _: Provider) -> Result<Option<Secret>> {
            Ok(self.0.lock().unwrap().clone().map(Secret::new))
        }
        fn set(&self, _: Provider, v: &Secret) -> Result<()> {
            *self.0.lock().unwrap() = Some(v.expose().to_string());
            Ok(())
        }
        fn delete(&self, _: Provider) -> Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn debug_never_reveals_secret() {
        let s = Secret::new(concat!("sk-", "proj-supersecret123456"));
        assert_eq!(format!("{s:?}"), "Secret(****)");
        assert!(!format!("{:?}", Some(s)).contains("supersecret"));
    }

    #[test]
    fn env_wins_over_keychain() {
        let store = MemStore::default();
        store
            .set(Provider::OpenAi, &Secret::new("from-keychain"))
            .unwrap();
        let creds = Credentials::new(
            Box::new(store),
            Box::new(|k| (k == "OPENAI_API_KEY").then(|| "from-env".to_string())),
        );
        assert_eq!(creds.get(Provider::OpenAi).unwrap().expose(), "from-env");
        assert_eq!(creds.source(Provider::OpenAi), SecretSource::Environment);
    }

    #[test]
    fn falls_back_to_keychain_and_reports_missing() {
        let creds = Credentials::new(Box::new(MemStore::default()), Box::new(|_| None));
        assert_eq!(creds.source(Provider::Anthropic), SecretSource::Missing);
        creds
            .store()
            .set(Provider::Anthropic, &Secret::new("k"))
            .unwrap();
        assert_eq!(creds.source(Provider::Anthropic), SecretSource::Keychain);
    }

    #[test]
    fn redact_removes_key_like_tokens() {
        let msg = concat!(
            "Incorrect API key provided: sk-",
            "proj-abcdefgh12345 and ek_abcdefghijk"
        );
        let out = redact(msg);
        assert!(!out.contains("abcdefgh12345"));
        assert!(!out.contains("ek_abcdefghijk"));
    }
}
