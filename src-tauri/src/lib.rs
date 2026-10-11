//! Aplicación de escritorio JARVIS (Tauri 2).
//!
//! Esta capa es fina: expone a la interfaz los servicios de `jarvis-core` como
//! comandos Tauri y reenvía los eventos de confirmación. Las claves API se leen
//! aquí (entorno o almacén del sistema) y nunca cruzan hacia el WebView.

use std::path::PathBuf;
use std::sync::Arc;

use jarvis_core::approvals::{ApprovalEvent, ApprovalRequest};
use jarvis_core::audit::AuditEntry;
use jarvis_core::config::Config;
use jarvis_core::error::JarvisError;
use jarvis_core::executor::{Origin, ToolCall, ToolOutcome};
use jarvis_core::secrets::Credentials;
use jarvis_core::text_agent::TextReply;
use jarvis_core::transcripts::{TranscriptEntry, UsageRecord, UsageSummary};
use jarvis_core::{Jarvis, RealtimeSessionResponse, Status};
use serde::Serialize;
use serde_json::Value;
use tauri::{Emitter, Manager, State};

struct AppState {
    jarvis: Arc<Jarvis>,
    config_error: Option<String>,
}

#[derive(Serialize)]
struct AppStatus {
    status: Status,
    config_error: Option<String>,
    backend: &'static str,
}

type CmdResult<T> = Result<T, JarvisError>;

#[tauri::command]
fn app_status(state: State<'_, AppState>) -> AppStatus {
    AppStatus {
        status: state.jarvis.status(),
        config_error: state.config_error.clone(),
        backend: "tauri",
    }
}

/// Emite un secreto efímero de Realtime. La respuesta no se registra.
#[tauri::command]
async fn realtime_session(
    state: State<'_, AppState>,
    voice: Option<String>,
    carry_over: Option<String>,
) -> CmdResult<RealtimeSessionResponse> {
    state
        .jarvis
        .create_realtime_session(voice.as_deref(), carry_over.as_deref())
        .await
}

/// Ejecuta una herramienta solicitada por la sesión de voz. La validación,
/// la política de permisos y la confirmación se aplican en `jarvis-core`.
#[tauri::command]
async fn tool_invoke(
    state: State<'_, AppState>,
    call_id: String,
    name: String,
    arguments: Value,
) -> CmdResult<ToolOutcome> {
    Ok(state
        .jarvis
        .executor
        .invoke(ToolCall {
            call_id,
            name,
            arguments,
            origin: Origin::Voice,
        })
        .await)
}

#[tauri::command]
fn approval_decide(state: State<'_, AppState>, id: String, approved: bool) -> CmdResult<()> {
    state.jarvis.approvals.decide(&id, approved)
}

#[tauri::command]
fn approvals_pending(state: State<'_, AppState>) -> Vec<ApprovalRequest> {
    state.jarvis.approvals.pending()
}

#[tauri::command]
async fn text_send(
    state: State<'_, AppState>,
    message: String,
    recent_context: Option<String>,
) -> CmdResult<TextReply> {
    state
        .jarvis
        .text
        .send(&message, recent_context.as_deref())
        .await
}

#[tauri::command]
fn text_reset(state: State<'_, AppState>) {
    state.jarvis.text.reset();
}

#[tauri::command]
fn audit_recent(state: State<'_, AppState>, limit: Option<usize>) -> Vec<AuditEntry> {
    state.jarvis.audit.recent(limit.unwrap_or(20).min(200))
}

#[tauri::command]
fn transcript_save(
    state: State<'_, AppState>,
    session_id: String,
    entries: Vec<TranscriptEntry>,
) -> CmdResult<Option<String>> {
    state.jarvis.transcripts.save(&session_id, &entries)
}

#[tauri::command]
fn transcripts_clear(state: State<'_, AppState>) -> usize {
    state.jarvis.transcripts.clear()
}

#[tauri::command]
fn usage_record(state: State<'_, AppState>, record: UsageRecord) -> CmdResult<()> {
    state.jarvis.usage.record(record)
}

#[tauri::command]
fn usage_summary(state: State<'_, AppState>) -> UsageSummary {
    state.jarvis.usage.summary()
}

fn build_state(config_dir: Option<PathBuf>, data_dir: Option<PathBuf>) -> AppState {
    let config_file = std::env::var_os("JARVIS_CONFIG_FILE")
        .map(PathBuf::from)
        .or_else(|| config_dir.map(|d| d.join("config.toml")));
    let (config, config_error) = match Config::load(config_file.as_deref()) {
        Ok(c) => (c, None),
        Err(e) => {
            tracing::error!("configuración no válida: {}", e.message);
            (
                Config::from_lookup(|_| None).expect("valores por defecto válidos"),
                Some(e.message),
            )
        }
    };
    AppState {
        jarvis: Arc::new(Jarvis::new(config, Credentials::system(), data_dir)),
        config_error,
    }
}

/// En Linux (WebKitGTK) hay que habilitar explícitamente WebRTC y conceder el
/// permiso de micrófono, que por defecto se deniega. Solo se autoriza audio
/// (nunca cámara) y solo para el contenido propio de la aplicación; el
/// micrófono únicamente se abre cuando el usuario inicia una conversación.
#[cfg(target_os = "linux")]
fn enable_linux_media(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        use webkit2gtk::glib::object::Cast;
        use webkit2gtk::{
            PermissionRequestExt, SettingsExt, UserMediaPermissionRequest, WebViewExt,
        };
        let wv = webview.inner();
        if let Some(settings) = WebViewExt::settings(&wv) {
            settings.set_enable_media_stream(true);
            settings.set_enable_webrtc(true);
            settings.set_enable_webaudio(true);
        }
        wv.connect_permission_request(|_, request| {
            if let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() {
                use webkit2gtk::UserMediaPermissionRequestExt;
                if media.is_for_audio_device() && !media.is_for_video_device() {
                    request.allow();
                } else {
                    request.deny();
                }
                return true;
            }
            false
        });
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Solo en desarrollo: permite un `.env` local (ignorado por git).
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,jarvis_core=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let state = build_state(
                app.path().app_config_dir().ok(),
                app.path().app_data_dir().ok(),
            );
            let handle = app.handle().clone();
            state
                .jarvis
                .approvals
                .set_notifier(Arc::new(move |ev: ApprovalEvent| {
                    let _ = handle.emit("jarvis://approval", ev);
                }));
            app.manage(state);
            #[cfg(target_os = "linux")]
            if let Some(w) = app.get_webview_window("main") {
                enable_linux_media(&w);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            realtime_session,
            tool_invoke,
            approval_decide,
            approvals_pending,
            text_send,
            text_reset,
            audit_recent,
            transcript_save,
            transcripts_clear,
            usage_record,
            usage_summary
        ])
        .run(tauri::generate_context!())
        .expect("error al ejecutar JARVIS");
}
