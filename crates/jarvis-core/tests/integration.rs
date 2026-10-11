//! Pruebas de integración de la capa de orquestación con una Vault real en
//! disco (temporal) y proveedores simulados con `wiremock`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use jarvis_core::approvals::ApprovalEvent;
use jarvis_core::config::Config;
use jarvis_core::error::{ErrorCode, Result};
use jarvis_core::executor::{Origin, OutcomeStatus, ToolCall};
use jarvis_core::secrets::{Credentials, Provider, Secret, SecretStore};
use jarvis_core::Jarvis;
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

struct NoStore;
impl SecretStore for NoStore {
    fn get(&self, _: Provider) -> Result<Option<Secret>> {
        Ok(None)
    }
    fn set(&self, _: Provider, _: &Secret) -> Result<()> {
        Ok(())
    }
    fn delete(&self, _: Provider) -> Result<()> {
        Ok(())
    }
}

fn seed_vault(root: &Path) {
    fs::create_dir_all(root.join("Proyectos")).unwrap();
    fs::create_dir_all(root.join("Diario")).unwrap();
    fs::create_dir_all(root.join(".obsidian")).unwrap();
    fs::write(
        root.join(".obsidian/workspace.md"),
        "reunión secreta interna",
    )
    .unwrap();
    fs::write(
        root.join("Proyectos/Alfa.md"),
        "---\nstatus: activo\n---\n# Alfa\n\nObjetivo: lanzar la beta en noviembre.\n\nNotas de la reunión con Marta sobre el presupuesto.\n\n- [ ] Preparar demo 📅 2026-10-11\n- [ ] Revisar contrato 📅 2026-10-01\n- [x] Kickoff\n",
    )
    .unwrap();
    fs::write(
        root.join("Proyectos/Beta.md"),
        "# Beta\n- [ ] Diseñar API\n",
    )
    .unwrap();
    fs::write(
        root.join("Diario/2026-10-11.md"),
        "# 11 de octubre\nHoy: demo de Alfa y gimnasio.\n",
    )
    .unwrap();
    fs::write(root.join("Ideas.md"), "Ideas sueltas sin relación.\n").unwrap();
}

struct Harness {
    jarvis: Arc<Jarvis>,
    _dir: tempfile::TempDir,
    vault: std::path::PathBuf,
    approvals: Arc<Mutex<Vec<String>>>,
}

/// `approve`: Some(true/false) responde automáticamente; None deja expirar.
fn harness(extra: &[(&str, &str)], keys: &[(&str, &str)], approve: Option<bool>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    seed_vault(&vault);
    let mut map: HashMap<String, String> = HashMap::new();
    map.insert("JARVIS_VAULT_PATH".into(), vault.display().to_string());
    map.insert("JARVIS_APPROVAL_TIMEOUT_MS".into(), "300".into());
    for (k, v) in extra {
        map.insert(k.to_string(), v.to_string());
    }
    let cfg = Config::from_lookup(|k| map.get(k).cloned()).unwrap();
    let keys: HashMap<String, String> = keys
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let creds = Credentials::new(Box::new(NoStore), Box::new(move |k| keys.get(k).cloned()));
    let jarvis = Arc::new(Jarvis::new(cfg, creds, Some(dir.path().join("data"))));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let broker = Arc::downgrade(&jarvis.approvals);
    jarvis.approvals.set_notifier(Arc::new(move |ev| {
        if let ApprovalEvent::Requested { request } = ev {
            seen2.lock().unwrap().push(request.summary.clone());
            if let (Some(decision), Some(b)) = (approve, broker.upgrade()) {
                b.decide(&request.id, decision).unwrap();
            }
        }
    }));
    Harness {
        jarvis,
        _dir: dir,
        vault,
        approvals: seen,
    }
}

fn call(id: &str, name: &str, args: Value) -> ToolCall {
    ToolCall {
        call_id: id.into(),
        name: name.into(),
        arguments: args,
        origin: Origin::Voice,
    }
}

#[tokio::test]
async fn search_and_read_return_relevant_fragments_only() {
    let h = harness(&[], &[], Some(true));
    let out = h
        .jarvis
        .executor
        .invoke(call(
            "c1",
            "search_notes",
            json!({"query": "reunion presupuesto"}),
        ))
        .await;
    assert_eq!(out.status, OutcomeStatus::Ok, "{out:?}");
    let results = out.data.unwrap()["results"].as_array().unwrap().clone();
    assert_eq!(results[0]["path"], "Proyectos/Alfa.md");
    // Las carpetas ocultas (.obsidian) nunca se indexan.
    assert!(results
        .iter()
        .all(|r| !r["path"].as_str().unwrap().starts_with('.')));

    let read = h
        .jarvis
        .executor
        .invoke(call(
            "c2",
            "read_note",
            json!({"path": "Proyectos/Alfa.md", "query": "presupuesto", "max_chars": 300}),
        ))
        .await;
    let content = read.data.unwrap()["content"].as_str().unwrap().to_string();
    assert!(content.contains("presupuesto"));
    assert!(
        !content.contains("Preparar demo"),
        "solo fragmentos relevantes: {content}"
    );
}

#[tokio::test]
async fn tasks_projects_and_daily_summary() {
    let h = harness(&[], &[], Some(true));
    let ex = &h.jarvis.executor;
    let open = ex
        .invoke(call("t1", "list_tasks", json!({})))
        .await
        .data
        .unwrap();
    assert_eq!(open["tasks"].as_array().unwrap().len(), 3);
    let projects = ex
        .invoke(call("t2", "list_projects", json!({})))
        .await
        .data
        .unwrap();
    assert_eq!(projects["projects"][0]["name"], "Alfa");
    assert_eq!(projects["projects"][0]["status"], "activo");
    let summary = ex
        .invoke(call("t3", "daily_summary", json!({"date": "2026-10-11"})))
        .await
        .data
        .unwrap();
    assert_eq!(
        summary["due_today"][0]["text"],
        "Preparar demo 📅 2026-10-11"
    );
    assert_eq!(
        summary["overdue"][0]["text"],
        "Revisar contrato 📅 2026-10-01"
    );
    assert_eq!(summary["daily_note"]["path"], "Diario/2026-10-11.md");
}

#[tokio::test]
async fn write_requires_approval_and_is_verified() {
    let h = harness(&[], &[], Some(true));
    let out = h
        .jarvis
        .executor
        .invoke(call(
            "w1",
            "create_note",
            json!({"title": "Plan Q4", "content": "Puntos clave"}),
        ))
        .await;
    assert_eq!(out.status, OutcomeStatus::Ok, "{out:?}");
    assert!(out.verified);
    assert_eq!(h.approvals.lock().unwrap().len(), 1);
    let written = fs::read_to_string(h.vault.join("JARVIS/Plan Q4.md")).unwrap();
    assert!(written.contains("# Plan Q4") && written.contains("Puntos clave"));

    // Nunca sobrescribe.
    let dup = h
        .jarvis
        .executor
        .invoke(call(
            "w2",
            "create_note",
            json!({"title": "Plan Q4", "content": "otra cosa"}),
        ))
        .await;
    assert_eq!(dup.error.unwrap().code, ErrorCode::AlreadyExists);
    assert!(fs::read_to_string(h.vault.join("JARVIS/Plan Q4.md"))
        .unwrap()
        .contains("Puntos clave"));

    let task = h
        .jarvis
        .executor
        .invoke(call(
            "w3",
            "add_task",
            json!({"text": "Llamar a Marta", "due": "2026-10-15"}),
        ))
        .await;
    assert_eq!(task.status, OutcomeStatus::Ok, "{task:?}");
    let tasks = fs::read_to_string(h.vault.join("JARVIS/Tareas.md")).unwrap();
    assert!(tasks.contains("- [ ] Llamar a Marta 📅 2026-10-15"));
}

#[tokio::test]
async fn denied_or_expired_approval_does_not_write() {
    let h = harness(&[], &[], Some(false));
    let out = h
        .jarvis
        .executor
        .invoke(call(
            "d1",
            "add_task",
            json!({"text": "No debería existir"}),
        ))
        .await;
    assert_eq!(out.status, OutcomeStatus::Denied);
    assert_eq!(out.error.as_ref().unwrap().code, ErrorCode::ApprovalDenied);
    assert!(!h.vault.join("JARVIS/Tareas.md").exists());
    assert_eq!(out.for_model()["ok"], false);

    let h = harness(&[], &[], None);
    let out = h
        .jarvis
        .executor
        .invoke(call("d2", "add_task", json!({"text": "Tampoco"})))
        .await;
    assert_eq!(out.error.unwrap().code, ErrorCode::ApprovalTimeout);
    assert!(!h.vault.join("JARVIS/Tareas.md").exists());
    assert!(h.jarvis.approvals.pending().is_empty());
}

#[tokio::test]
async fn policy_can_allow_or_deny_writes() {
    let h = harness(&[("JARVIS_POLICY_WRITE", "allow")], &[], None);
    let out = h
        .jarvis
        .executor
        .invoke(call("p1", "add_task", json!({"text": "Directa"})))
        .await;
    assert_eq!(out.status, OutcomeStatus::Ok);
    assert!(h.approvals.lock().unwrap().is_empty());

    let h = harness(&[("JARVIS_POLICY_WRITE", "deny")], &[], Some(true));
    let out = h
        .jarvis
        .executor
        .invoke(call("p2", "add_task", json!({"text": "Prohibida"})))
        .await;
    assert_eq!(out.error.unwrap().code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn repeated_call_id_executes_once() {
    let h = harness(&[("JARVIS_POLICY_WRITE", "allow")], &[], None);
    let ex = h.jarvis.executor.clone();
    let c = call("same-id", "add_task", json!({"text": "Solo una vez"}));
    // Concurrentes (reintento durante una reconexión) y secuencial.
    let (a, b) = tokio::join!(ex.invoke(c.clone()), ex.invoke(c.clone()));
    let c3 = ex.invoke(c).await;
    assert_eq!(a.status, OutcomeStatus::Ok);
    assert_eq!(b.status, OutcomeStatus::Ok);
    assert!(a.deduplicated ^ b.deduplicated);
    assert!(c3.deduplicated);
    let tasks = fs::read_to_string(h.vault.join("JARVIS/Tareas.md")).unwrap();
    assert_eq!(tasks.matches("Solo una vez").count(), 1);
}

#[tokio::test]
async fn invalid_calls_are_rejected_and_audited() {
    let h = harness(&[], &[], Some(true));
    let ex = &h.jarvis.executor;
    let bad = [
        call("i1", "read_note", json!({"path": "../../etc/passwd"})),
        call(
            "i2",
            "search_notes",
            json!({"query": "x", "shell": "rm -rf /"}),
        ),
        call("i3", "delete_vault", json!({})),
        call("i4", "add_task", json!({"text": 12})),
        call("", "search_notes", json!({"query": "x"})),
    ];
    for c in bad {
        let out = ex.invoke(c.clone()).await;
        assert_eq!(out.status, OutcomeStatus::Error, "{c:?}");
    }
    assert!(
        h.approvals.lock().unwrap().is_empty(),
        "nada inválido llega a pedir confirmación"
    );
    let audit = h.jarvis.audit.recent(10);
    assert!(audit.iter().all(|e| e.status == "error"));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_escape_the_vault() {
    let h = harness(&[], &[], Some(true));
    let outside = h._dir.path().join("fuera.md");
    fs::write(&outside, "secreto").unwrap();
    std::os::unix::fs::symlink(&outside, h.vault.join("enlace.md")).unwrap();
    let out = h
        .jarvis
        .executor
        .invoke(call("s1", "read_note", json!({"path": "enlace.md"})))
        .await;
    assert_eq!(out.error.unwrap().code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn missing_vault_reports_clear_error() {
    let h = harness(&[("JARVIS_VAULT_PATH", "/no/existe")], &[], Some(true));
    let out = h
        .jarvis
        .executor
        .invoke(call("v1", "search_notes", json!({"query": "x"})))
        .await;
    assert_eq!(out.error.unwrap().code, ErrorCode::VaultNotConfigured);
    assert!(!h.jarvis.status().vault.ok);
}

// ------------------------------------------------------------ Realtime

#[tokio::test]
async fn realtime_session_uses_server_key_and_returns_ephemeral_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/realtime/client_secrets"))
        .and(header("authorization", "Bearer sk-test-long-lived"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": "ek_test_ephemeral",
            "expires_at": 1_900_000_000,
            "session": {"type": "realtime"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let h = harness(
        &[
            ("JARVIS_OPENAI_BASE_URL", &server.uri()),
            ("OPENAI_REALTIME_VOICE", "cedar"),
        ],
        &[("OPENAI_API_KEY", "sk-test-long-lived")],
        Some(true),
    );
    let resp = h
        .jarvis
        .create_realtime_session(None, Some("Usuario: hola"))
        .await
        .unwrap();
    assert_eq!(resp.session.ephemeral_key, "ek_test_ephemeral");
    assert_eq!(resp.session.settings.voice, "cedar");
    assert!(resp.instructions.contains("Usuario: hola"));
    assert!(resp.tools.iter().any(|t| t.name == "search_notes"));
    assert!(
        !resp.tools.iter().any(|t| t.name == "delegate_to_claude"),
        "sin Claude no se ofrece delegación"
    );

    let serialized = serde_json::to_string(&resp).unwrap();
    assert!(
        !serialized.contains("sk-test-long-lived"),
        "la clave larga nunca sale hacia la interfaz"
    );
    assert!(
        !format!("{:?}", resp.session).contains("ek_test_ephemeral"),
        "Debug oculta el secreto"
    );

    let req: &Request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["session"]["type"], "realtime");
    assert_eq!(body["session"]["model"], "gpt-realtime-2.1");
    assert_eq!(body["session"]["audio"]["output"]["voice"], "cedar");
    assert_eq!(
        body["session"]["audio"]["input"]["turn_detection"]["interrupt_response"],
        true
    );
    assert_eq!(
        body["session"]["audio"]["input"]["transcription"]["language"],
        "es"
    );
    assert_eq!(body["expires_after"]["seconds"], 600);
}

#[tokio::test]
async fn realtime_errors_are_typed() {
    let h = harness(&[], &[], Some(true));
    let err = h
        .jarvis
        .create_realtime_session(None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::MissingApiKey);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"code": "model_not_found", "message": "The model `gpt-x` does not exist"}
        })))
        .mount(&server)
        .await;
    let h = harness(
        &[("JARVIS_OPENAI_BASE_URL", &server.uri())],
        &[("OPENAI_API_KEY", "sk-x")],
        Some(true),
    );
    let err = h
        .jarvis
        .create_realtime_session(None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelUnavailable);

    let err = h
        .jarvis
        .create_realtime_session(Some("voz-inventada"), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Config);

    let h = harness(
        &[("JARVIS_OPENAI_BASE_URL", "http://127.0.0.1:9")],
        &[("OPENAI_API_KEY", "sk-x")],
        Some(true),
    );
    let err = h
        .jarvis
        .create_realtime_session(None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NetworkError);
    assert!(err.retryable);
}

// --------------------------------------------------------------- Claude

#[tokio::test]
async fn delegation_sends_only_requested_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Conclusión: el presupuesto es viable."}],
            "stop_reason": "end_turn"
        })))
        .mount(&server)
        .await;
    let h = harness(
        &[("JARVIS_ANTHROPIC_BASE_URL", &server.uri())],
        &[("ANTHROPIC_API_KEY", "sk-ant-test")],
        Some(true),
    );
    let out = h
        .jarvis
        .executor
        .invoke(call(
            "del1",
            "delegate_to_claude",
            json!({"task": "Analiza el presupuesto", "note_paths": ["Proyectos/Alfa.md"]}),
        ))
        .await;
    assert_eq!(out.status, OutcomeStatus::Ok, "{out:?}");
    assert!(out.data.unwrap()["answer"]
        .as_str()
        .unwrap()
        .contains("viable"));
    let req = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let prompt = body["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("presupuesto"));
    assert!(
        !prompt.contains("Ideas sueltas"),
        "no se envía la Vault completa"
    );
}

#[tokio::test]
async fn text_interface_with_claude_runs_tools() {
    let server = MockServer::start().await;
    let turn = Arc::new(Mutex::new(0));
    let turn2 = turn.clone();
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(move |_: &Request| {
            let mut t = turn2.lock().unwrap();
            *t += 1;
            if *t == 1 {
                ResponseTemplate::new(200).set_body_json(json!({
                    "content": [{"type": "tool_use", "id": "toolu_1", "name": "list_tasks", "input": {"due": "overdue"}}],
                    "stop_reason": "tool_use"
                }))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({
                    "content": [{"type": "text", "text": "Tienes una tarea atrasada: revisar el contrato."}],
                    "stop_reason": "end_turn"
                }))
            }
        })
        .mount(&server)
        .await;
    let h = harness(
        &[("JARVIS_ANTHROPIC_BASE_URL", &server.uri())],
        &[("ANTHROPIC_API_KEY", "sk-ant-test")],
        Some(true),
    );
    let reply = h
        .jarvis
        .text
        .send("¿Qué tengo atrasado?", None)
        .await
        .unwrap();
    assert_eq!(reply.provider, "claude");
    assert_eq!(reply.tools[0].tool, "list_tasks");
    assert!(reply.reply.contains("atrasada"));
    let reqs = server.received_requests().await.unwrap();
    let second: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    let tool_result = &second["messages"].as_array().unwrap().last().unwrap()["content"][0];
    assert_eq!(tool_result["type"], "tool_result");
    assert!(tool_result["content"]
        .as_str()
        .unwrap()
        .contains("Revisar contrato"));
}

#[tokio::test]
async fn text_interface_works_without_any_credentials() {
    let h = harness(&[], &[], Some(true));
    assert_eq!(h.jarvis.text.provider(), "commands");
    let r = h.jarvis.text.send("/tareas vencidas", None).await.unwrap();
    assert!(r.reply.contains("Revisar contrato"), "{}", r.reply);
    let r = h
        .jarvis
        .text
        .send("/tarea Comprar café @2026-10-20", None)
        .await
        .unwrap();
    assert!(r.reply.contains("verificada"), "{}", r.reply);
    let r = h.jarvis.text.send("hola", None).await.unwrap();
    assert!(r.reply.contains("/buscar"));
    let s = h.jarvis.status();
    assert!(!s.voice_available);
    assert!(!s.limitations.is_empty());
}
