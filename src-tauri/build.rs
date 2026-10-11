// Declara los comandos de la app para que solo los listados en
// `capabilities/default.json` puedan invocarse desde la interfaz.
const COMMANDS: &[&str] = &[
    "app_status",
    "realtime_session",
    "tool_invoke",
    "approval_decide",
    "approvals_pending",
    "text_send",
    "text_reset",
    "audit_recent",
    "transcript_save",
    "transcripts_clear",
    "usage_record",
    "usage_summary",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("error al preparar la compilación de Tauri");
}
