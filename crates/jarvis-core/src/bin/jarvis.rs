//! CLI de soporte de JARVIS.
//!
//!   jarvis secrets set openai|anthropic   Guarda una clave en el almacén del sistema (entrada oculta)
//!   jarvis secrets delete openai|anthropic
//!   jarvis secrets status                 Indica de dónde se lee cada clave, sin mostrarla
//!   jarvis doctor                         Comprueba configuración, Vault, claves y modelo Realtime
//!   jarvis tool <nombre> '<json>'         Ejecuta una herramienta con la misma política que la app
//!   jarvis text "<mensaje>"               Envía un mensaje a la interfaz de texto

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use jarvis_core::approvals::ApprovalEvent;
use jarvis_core::config::Config;
use jarvis_core::executor::{Origin, ToolCall};
use jarvis_core::secrets::{Credentials, Provider, Secret, SecretSource};
use jarvis_core::Jarvis;

fn usage() -> ExitCode {
    eprintln!(
        "Uso:\n  jarvis secrets set|delete openai|anthropic\n  jarvis secrets status\n  jarvis doctor\n  jarvis tool <nombre> '<json>'\n  jarvis text \"<mensaje>\""
    );
    ExitCode::from(2)
}

fn config_file() -> Option<PathBuf> {
    std::env::var_os("JARVIS_CONFIG_FILE").map(PathBuf::from)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["secrets", "set", p] => secrets_set(p),
        ["secrets", "delete", p] => secrets_delete(p),
        ["secrets", "status"] => {
            let c = Credentials::system();
            for p in [Provider::OpenAi, Provider::Anthropic] {
                println!("{:<10} {}", p.keychain_user(), describe(c.source(p)));
            }
            ExitCode::SUCCESS
        }
        ["doctor"] => doctor().await,
        ["tool", name, json] => run_tool(name, json).await,
        ["text", msg] => run_text(msg).await,
        _ => usage(),
    }
}

fn describe(s: SecretSource) -> &'static str {
    match s {
        SecretSource::Environment => "configurada (variable de entorno)",
        SecretSource::Keychain => "configurada (almacén del sistema)",
        SecretSource::Missing => "no configurada",
    }
}

fn secrets_set(p: &str) -> ExitCode {
    let Some(provider) = Provider::parse(p) else {
        return usage();
    };
    let value = match rpassword::prompt_password(format!("Clave de {p} (no se mostrará): ")) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("No se ha introducido ninguna clave.");
            return ExitCode::FAILURE;
        }
    };
    let c = Credentials::system();
    match c.store().set(provider, &Secret::new(value.trim())) {
        Ok(()) => {
            println!("Clave de {p} guardada en el almacén de credenciales del sistema.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.message);
            ExitCode::FAILURE
        }
    }
}

fn secrets_delete(p: &str) -> ExitCode {
    let Some(provider) = Provider::parse(p) else {
        return usage();
    };
    match Credentials::system().store().delete(provider) {
        Ok(()) => {
            println!("Clave de {p} eliminada del almacén del sistema.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.message);
            ExitCode::FAILURE
        }
    }
}

fn load() -> Result<Jarvis, ExitCode> {
    match Config::load(config_file().as_deref()) {
        Ok(cfg) => Ok(Jarvis::new(cfg, Credentials::system(), None)),
        Err(e) => {
            eprintln!("Configuración no válida: {}", e.message);
            Err(ExitCode::FAILURE)
        }
    }
}

async fn doctor() -> ExitCode {
    let j = match load() {
        Ok(j) => j,
        Err(c) => return c,
    };
    let s = j.status();
    let mut ok = true;
    println!("Modelo Realtime:   {}", s.realtime_model);
    println!("Voz por defecto:   {}", s.default_voice);
    println!("Clave OpenAI:      {}", describe(s.openai_key));
    println!(
        "Clave Anthropic:   {} (opcional)",
        describe(s.anthropic_key)
    );
    println!("Interfaz de texto: {}", s.text_provider);
    match (&s.vault.path, &s.vault.error) {
        (Some(p), _) => println!("Vault:             {p}"),
        (_, Some(e)) => {
            ok = false;
            println!("Vault:             ERROR — {e}")
        }
        _ => {}
    }
    if s.voice_available {
        match j.check_realtime_model().await {
            Ok(()) => println!("Modelo Realtime:   disponible para esta cuenta"),
            Err(e) => {
                ok = false;
                println!("Modelo Realtime:   ERROR {:?} — {}", e.code, e.message)
            }
        }
    } else {
        ok = false;
        println!("Voz real:          no disponible (falta la clave de OpenAI)");
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Las confirmaciones se piden por terminal.
fn terminal_approvals(j: &Jarvis) {
    let broker = Arc::downgrade(&j.approvals);
    j.approvals.set_notifier(Arc::new(move |ev| {
        if let ApprovalEvent::Requested { request } = ev {
            let broker = broker.clone();
            std::thread::spawn(move || {
                eprint!(
                    "\n¿Autorizas? {}\n{}[s/N]: ",
                    request.summary,
                    request
                        .preview
                        .map(|p| format!("{p}\n"))
                        .unwrap_or_default()
                );
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                let _ = std::io::stdin().lock().read_line(&mut line);
                let yes = matches!(
                    line.trim().to_lowercase().as_str(),
                    "s" | "si" | "sí" | "y" | "yes"
                );
                if let Some(b) = broker.upgrade() {
                    let _ = b.decide(&request.id, yes);
                }
            });
        }
    }));
}

async fn run_tool(name: &str, json: &str) -> ExitCode {
    let j = match load() {
        Ok(j) => j,
        Err(c) => return c,
    };
    terminal_approvals(&j);
    let arguments = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("JSON no válido: {e}");
            return ExitCode::from(2);
        }
    };
    let outcome = j
        .executor
        .invoke(ToolCall {
            call_id: format!("cli-{}", uuid::Uuid::new_v4()),
            name: name.into(),
            arguments,
            origin: Origin::Test,
        })
        .await;
    println!(
        "{}",
        serde_json::to_string_pretty(&outcome).unwrap_or_default()
    );
    if outcome.status == jarvis_core::executor::OutcomeStatus::Ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

async fn run_text(msg: &str) -> ExitCode {
    let j = match load() {
        Ok(j) => j,
        Err(c) => return c,
    };
    terminal_approvals(&j);
    match j.text.send(msg, None).await {
        Ok(r) => {
            println!("[{}]\n{}", r.provider, r.reply);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{:?}: {}", e.code, e.message);
            ExitCode::FAILURE
        }
    }
}
