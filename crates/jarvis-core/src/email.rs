//! Lectura de correo por IMAP, **solo lectura**.
//!
//! - El buzón se abre con `EXAMINE` (modo solo lectura) y los mensajes se piden
//!   con `BODY.PEEK[]`, de modo que leerlos no los marca como leídos.
//! - No hay ninguna operación para enviar, mover, marcar ni borrar correo.
//! - La contraseña es una **contraseña de aplicación** guardada en el almacén
//!   del sistema (`jarvis secrets set email`), nunca la contraseña principal.
//! - El contenido de los correos es no confiable: se devuelve como datos.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_imap::types::Flag;
use futures::TryStreamExt;
use mail_parser::{Address, MessageParser, MimeHeaders};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::rustls;

use crate::config::EmailConfig;
use crate::error::{ErrorCode, JarvisError, Result};
use crate::secrets::Secret;
use crate::vault::truncate_chars;
use crate::web::{html_to_text, UNTRUSTED_NOTICE};

const MAX_MESSAGE_BYTES: u32 = 10_000_000;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
pub struct EmailSummary {
    pub id: u32,
    pub from: String,
    pub subject: String,
    pub date: Option<String>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmailMessage {
    pub id: u32,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: Option<String>,
    pub body: String,
    pub truncated: bool,
    pub attachments: Vec<String>,
    pub notice: &'static str,
}

/// Flujo TCP con o sin TLS (sin TLS solo en pruebas contra localhost).
enum Conn {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Conn::Plain(_) => "Conn::Plain",
            Conn::Tls(_) => "Conn::Tls",
        })
    }
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

type Session = async_imap::Session<Conn>;

pub struct EmailClient {
    cfg: EmailConfig,
}

fn imap_err(e: async_imap::error::Error) -> JarvisError {
    JarvisError::new(
        ErrorCode::ProviderError,
        format!(
            "Error del servidor de correo: {}",
            crate::secrets::redact(&e.to_string())
        ),
    )
}

fn io_err(e: io::Error) -> JarvisError {
    JarvisError::new(
        ErrorCode::NetworkError,
        format!("No se pudo conectar con el servidor de correo: {e}"),
    )
}

impl EmailClient {
    /// `None` si el correo no está configurado (falta la dirección).
    pub fn new(cfg: &EmailConfig) -> Option<Self> {
        cfg.address.as_ref()?;
        Some(Self { cfg: cfg.clone() })
    }

    pub async fn list(
        &self,
        password: Option<&Secret>,
        unread_only: bool,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EmailSummary>> {
        let mut s = self.connect(password).await?;
        let r = tokio::time::timeout(
            TIMEOUT,
            list_in(&mut s, &self.cfg.mailbox, unread_only, query, limit),
        )
        .await
        .map_err(|_| {
            JarvisError::new(ErrorCode::Timeout, "El servidor de correo tardó demasiado.")
        })?;
        let _ = s.logout().await;
        r
    }

    pub async fn read(
        &self,
        password: Option<&Secret>,
        id: u32,
        max_chars: usize,
    ) -> Result<EmailMessage> {
        let mut s = self.connect(password).await?;
        let r = tokio::time::timeout(TIMEOUT, read_in(&mut s, &self.cfg.mailbox, id, max_chars))
            .await
            .map_err(|_| {
                JarvisError::new(ErrorCode::Timeout, "El servidor de correo tardó demasiado.")
            })?;
        let _ = s.logout().await;
        r
    }

    async fn connect(&self, password: Option<&Secret>) -> Result<Session> {
        let password = password.ok_or_else(|| {
            JarvisError::new(
                ErrorCode::MissingApiKey,
                "Falta la contraseña de aplicación del correo. Guárdala con `jarvis secrets set email`.",
            )
        })?;
        let address = self.cfg.address.clone().unwrap_or_default();
        let host = self.cfg.host().ok_or_else(|| {
            JarvisError::new(
                ErrorCode::Config,
                "No sé qué servidor de correo usar. Indica JARVIS_EMAIL_IMAP_HOST.",
            )
        })?;
        let fut = async {
            let tcp = TcpStream::connect((host.as_str(), self.cfg.imap_port))
                .await
                .map_err(io_err)?;
            let local = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
            let conn = if self.cfg.insecure_plaintext && local {
                Conn::Plain(tcp)
            } else {
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .map_err(|_| JarvisError::internal("TLS"))?
                .with_root_certificates(roots)
                .with_no_client_auth();
                let name = rustls::pki_types::ServerName::try_from(host.clone())
                    .map_err(|_| JarvisError::new(ErrorCode::Config, "Servidor IMAP no válido."))?;
                let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                    .connect(name, tcp)
                    .await
                    .map_err(io_err)?;
                Conn::Tls(Box::new(tls))
            };
            let mut client = async_imap::Client::new(conn);
            client.read_response().await.map_err(io_err)?;
            client
                .login(&address, password.expose())
                .await
                .map_err(|(e, _)| {
                    tracing::warn!("inicio de sesión IMAP rechazado");
                    let _ = e;
                    JarvisError::new(
                        ErrorCode::InvalidApiKey,
                        "El servidor de correo rechazó el usuario o la contraseña de aplicación.",
                    )
                })
        };
        tokio::time::timeout(TIMEOUT, fut).await.map_err(|_| {
            JarvisError::new(ErrorCode::Timeout, "El servidor de correo no respondió.")
        })?
    }
}

/// Deja solo caracteres seguros para una búsqueda IMAP entre comillas.
pub fn sanitize_query(q: &str) -> String {
    let cleaned: String = q
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '@' | '.' | '_' | '-' | '+'))
        .take(100)
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn list_in(
    s: &mut Session,
    mailbox: &str,
    unread_only: bool,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<EmailSummary>> {
    s.examine(mailbox).await.map_err(imap_err)?;
    let base = if unread_only { "UNSEEN" } else { "ALL" };
    let search = match query.map(sanitize_query).filter(|q| !q.is_empty()) {
        Some(q) => format!("CHARSET UTF-8 {base} OR FROM \"{q}\" SUBJECT \"{q}\""),
        None => base.to_string(),
    };
    let mut uids: Vec<u32> = s
        .uid_search(search)
        .await
        .map_err(imap_err)?
        .into_iter()
        .collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit);
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let set = uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let fetches: Vec<_> = s
        .uid_fetch(set, "(UID FLAGS RFC822.HEADER)")
        .await
        .map_err(imap_err)?
        .try_collect()
        .await
        .map_err(imap_err)?;
    let parser = MessageParser::default();
    let mut out: Vec<EmailSummary> = fetches
        .iter()
        .filter_map(|f| {
            let uid = f.uid?;
            let msg = parser.parse(f.header()?)?;
            Some(EmailSummary {
                id: uid,
                from: format_address(msg.from()),
                subject: msg.subject().unwrap_or("(sin asunto)").to_string(),
                date: msg.date().map(|d| d.to_rfc3339()),
                unread: !f.flags().any(|fl| matches!(fl, Flag::Seen)),
            })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.id));
    Ok(out)
}

async fn read_in(
    s: &mut Session,
    mailbox: &str,
    id: u32,
    max_chars: usize,
) -> Result<EmailMessage> {
    s.examine(mailbox).await.map_err(imap_err)?;
    let not_found =
        || JarvisError::new(ErrorCode::NotFound, format!("No encontré el correo {id}."));
    let sizes: Vec<_> = s
        .uid_fetch(id.to_string(), "(UID RFC822.SIZE)")
        .await
        .map_err(imap_err)?
        .try_collect()
        .await
        .map_err(imap_err)?;
    let size = sizes
        .iter()
        .find(|f| f.uid == Some(id))
        .ok_or_else(not_found)?
        .size
        .unwrap_or(0);
    if size > MAX_MESSAGE_BYTES {
        return Err(JarvisError::new(
            ErrorCode::InvalidArguments,
            "El correo es demasiado grande para leerlo.",
        ));
    }
    let fetches: Vec<_> = s
        .uid_fetch(id.to_string(), "(UID BODY.PEEK[])")
        .await
        .map_err(imap_err)?
        .try_collect()
        .await
        .map_err(imap_err)?;
    let raw = fetches
        .iter()
        .find(|f| f.uid == Some(id))
        .and_then(|f| f.body())
        .ok_or_else(not_found)?;
    let msg = MessageParser::default().parse(raw).ok_or_else(|| {
        JarvisError::new(
            ErrorCode::ProviderError,
            "No se pudo interpretar el correo.",
        )
    })?;
    let body = match msg.body_text(0) {
        Some(t) => t.into_owned(),
        None => msg
            .body_html(0)
            .map(|h| html_to_text(h.as_bytes()).1)
            .unwrap_or_default(),
    };
    let (body, truncated) = truncate_chars(body.trim(), max_chars);
    Ok(EmailMessage {
        id,
        from: format_address(msg.from()),
        to: format_address(msg.to()),
        subject: msg.subject().unwrap_or("(sin asunto)").to_string(),
        date: msg.date().map(|d| d.to_rfc3339()),
        body,
        truncated,
        attachments: msg
            .attachments()
            .filter_map(|a| a.attachment_name().map(str::to_string))
            .take(20)
            .collect(),
        notice: UNTRUSTED_NOTICE,
    })
}

fn format_address(a: Option<&Address<'_>>) -> String {
    let Some(first) = a.and_then(|a| a.first()) else {
        return "(desconocido)".into();
    };
    match (first.name(), first.address()) {
        (Some(n), Some(e)) => format!("{n} <{e}>"),
        (Some(n), None) => n.to_string(),
        (None, Some(e)) => e.to_string(),
        (None, None) => "(desconocido)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_search_terms() {
        assert_eq!(
            sanitize_query("factura \"x\" OR ALL\r\nDELETE"),
            "factura x OR ALLDELETE"
        );
        assert_eq!(sanitize_query("  maría@ejemplo.es  "), "maría@ejemplo.es");
    }

    #[test]
    fn deduces_imap_host() {
        let mut c = crate::config::Config::from_lookup(|_| None).unwrap().email;
        c.address = Some("yo@gmail.com".into());
        assert_eq!(c.host().as_deref(), Some("imap.gmail.com"));
        c.address = Some("yo@icloud.com".into());
        assert_eq!(c.host().as_deref(), Some("imap.mail.me.com"));
        c.address = Some("yo@midominio.dev".into());
        assert_eq!(c.host(), None);
    }
}
