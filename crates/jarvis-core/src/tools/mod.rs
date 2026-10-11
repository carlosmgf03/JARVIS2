//! Catálogo de herramientas de JARVIS.
//!
//! Fuente única de verdad: nombre, descripción, esquema JSON de parámetros y
//! nivel de riesgo de cada herramienta se definen aquí. La interfaz de voz
//! (Realtime) y la interfaz de texto (Claude o modo comandos) obtienen el
//! catálogo de esta capa; ninguna define herramientas por su cuenta.
//!
//! La validación es estricta: campos desconocidos, tipos incorrectos, valores
//! fuera de rango o rutas inseguras se rechazan con `invalid_arguments` antes
//! de evaluar la política o tocar la Vault.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{JarvisError, Result};
use crate::policy::RiskLevel;
use crate::vault::{check_relative, DueFilter, TaskFilter, TaskStatus};

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub risk: RiskLevel,
}

pub const SEARCH_NOTES: &str = "search_notes";
pub const READ_NOTE: &str = "read_note";
pub const LIST_TASKS: &str = "list_tasks";
pub const LIST_PROJECTS: &str = "list_projects";
pub const CREATE_NOTE: &str = "create_note";
pub const ADD_TASK: &str = "add_task";
pub const DAILY_SUMMARY: &str = "daily_summary";
pub const DELEGATE_TO_CLAUDE: &str = "delegate_to_claude";

/// Catálogo completo. `include_delegate` es falso si Claude no está configurado.
pub fn catalog(include_delegate: bool) -> Vec<ToolSpec> {
    let mut tools = vec![
        ToolSpec {
            name: SEARCH_NOTES,
            description: "Busca notas en la Vault de Obsidian del usuario. Devuelve rutas, títulos y fragmentos breves. Úsala antes de responder preguntas que dependan de sus notas, proyectos o memoria personal.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Palabras clave a buscar.", "minLength": 1, "maxLength": 200},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 10, "description": "Número máximo de resultados (por defecto 5)."}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            risk: RiskLevel::Read,
        },
        ToolSpec {
            name: READ_NOTE,
            description: "Lee una nota de la Vault por su ruta relativa (obtenida de search_notes). Si se indica 'query', devuelve solo los fragmentos relevantes.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Ruta relativa a la Vault, p. ej. 'Proyectos/Alfa.md'.", "minLength": 1, "maxLength": 400},
                    "query": {"type": "string", "description": "Tema a localizar dentro de la nota.", "maxLength": 200},
                    "max_chars": {"type": "integer", "minimum": 200, "maximum": 4000, "description": "Máximo de caracteres a devolver (por defecto 2000)."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            risk: RiskLevel::Read,
        },
        ToolSpec {
            name: LIST_TASKS,
            description: "Consulta tareas de la Vault (casillas '- [ ]' de Obsidian). Permite filtrar por estado, texto, proyecto y vencimiento.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["open", "done", "all"], "description": "Por defecto 'open'."},
                    "query": {"type": "string", "maxLength": 200},
                    "project": {"type": "string", "maxLength": 120, "description": "Nombre del proyecto o carpeta."},
                    "due": {"type": "string", "enum": ["any", "today", "overdue", "week"], "description": "Por defecto 'any'."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "additionalProperties": false
            }),
            risk: RiskLevel::Read,
        },
        ToolSpec {
            name: LIST_PROJECTS,
            description: "Lista los proyectos del usuario (notas en la carpeta de proyectos o con 'type: project') con su número de tareas abiertas.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 30}
                },
                "additionalProperties": false
            }),
            risk: RiskLevel::Read,
        },
        ToolSpec {
            name: DAILY_SUMMARY,
            description: "Resumen del día: nota diaria, tareas que vencen hoy, tareas atrasadas y notas modificadas.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "date": {"type": "string", "description": "Fecha AAAA-MM-DD. Por defecto, hoy.", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}
                },
                "additionalProperties": false
            }),
            risk: RiskLevel::Read,
        },
        ToolSpec {
            name: CREATE_NOTE,
            description: "Crea una nota nueva en la Vault. Nunca sobrescribe una nota existente. Puede requerir confirmación del usuario en pantalla.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 1, "maxLength": 120},
                    "content": {"type": "string", "maxLength": 20000, "description": "Contenido en Markdown."},
                    "folder": {"type": "string", "maxLength": 200, "description": "Carpeta relativa a la Vault. Por defecto la carpeta de notas de JARVIS."}
                },
                "required": ["title", "content"],
                "additionalProperties": false
            }),
            risk: RiskLevel::Write,
        },
        ToolSpec {
            name: ADD_TASK,
            description: "Añade una tarea a la lista de tareas de JARVIS en la Vault. Puede requerir confirmación del usuario en pantalla.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "minLength": 1, "maxLength": 300, "description": "Una sola línea."},
                    "due": {"type": "string", "description": "Fecha de vencimiento AAAA-MM-DD.", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}
                },
                "required": ["text"],
                "additionalProperties": false
            }),
            risk: RiskLevel::Write,
        },
    ];
    if include_delegate {
        tools.push(ToolSpec {
            name: DELEGATE_TO_CLAUDE,
            description: "Delega en Claude una tarea que requiera razonamiento extenso, análisis de documentos largos o programación. Envía solo la tarea y el contexto imprescindible. No la uses para conversación normal ni preguntas sencillas.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {"type": "string", "minLength": 1, "maxLength": 4000, "description": "Qué debe hacer Claude, de forma autocontenida."},
                    "context": {"type": "string", "maxLength": 8000, "description": "Contexto mínimo necesario."},
                    "note_paths": {"type": "array", "items": {"type": "string", "maxLength": 400}, "maxItems": 3, "description": "Notas de la Vault cuyos fragmentos relevantes se adjuntarán."}
                },
                "required": ["task"],
                "additionalProperties": false
            }),
            risk: RiskLevel::Delegate,
        });
    }
    tools
}

pub fn spec(name: &str, include_delegate: bool) -> Option<ToolSpec> {
    catalog(include_delegate)
        .into_iter()
        .find(|t| t.name == name)
}

// ------------------------------------------------------------ argumentos

#[derive(Debug, Clone, PartialEq)]
pub enum ToolArgs {
    SearchNotes {
        query: String,
        limit: usize,
    },
    ReadNote {
        path: String,
        query: Option<String>,
        max_chars: usize,
    },
    ListTasks(TaskFilterArgs),
    ListProjects {
        limit: usize,
    },
    DailySummary {
        date: Option<NaiveDate>,
    },
    CreateNote {
        title: String,
        content: String,
        folder: Option<String>,
    },
    AddTask {
        text: String,
        due: Option<NaiveDate>,
    },
    DelegateToClaude {
        task: String,
        context: Option<String>,
        note_paths: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskFilterArgs {
    pub status: &'static str,
    pub query: Option<String>,
    pub project: Option<String>,
    pub due: &'static str,
    pub limit: usize,
}

impl TaskFilterArgs {
    pub fn to_filter(&self) -> TaskFilter {
        TaskFilter {
            status: match self.status {
                "done" => TaskStatus::Done,
                "all" => TaskStatus::All,
                _ => TaskStatus::Open,
            },
            query: self.query.clone(),
            project: self.project.clone(),
            due: match self.due {
                "today" => DueFilter::Today,
                "overdue" => DueFilter::Overdue,
                "week" => DueFilter::Week,
                _ => DueFilter::Any,
            },
            limit: self.limit,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchRaw {
    query: String,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRaw {
    path: String,
    query: Option<String>,
    max_chars: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TasksRaw {
    status: Option<String>,
    query: Option<String>,
    project: Option<String>,
    due: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitRaw {
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyRaw {
    date: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRaw {
    title: String,
    content: String,
    folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskRaw {
    text: String,
    due: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateRaw {
    task: String,
    context: Option<String>,
    note_paths: Option<Vec<String>>,
}

/// Valida nombre y argumentos de una llamada. Acepta un objeto JSON o una
/// cadena con JSON (formato en que Realtime entrega los argumentos).
pub fn parse_args(name: &str, raw: &Value, include_delegate: bool) -> Result<ToolArgs> {
    if spec(name, include_delegate).is_none() {
        return Err(JarvisError::new(
            crate::error::ErrorCode::UnknownTool,
            format!(
                "Herramienta desconocida o no disponible: «{}».",
                name.chars().take(60).collect::<String>()
            ),
        ));
    }
    let value = match raw {
        Value::String(s) if s.trim().is_empty() => json!({}),
        Value::String(s) => serde_json::from_str(s)
            .map_err(|_| JarvisError::invalid("Los argumentos no son JSON válido."))?,
        Value::Null => json!({}),
        v => v.clone(),
    };
    if !value.is_object() {
        return Err(JarvisError::invalid(
            "Los argumentos deben ser un objeto JSON.",
        ));
    }

    fn de<T: for<'a> Deserialize<'a>>(v: Value) -> Result<T> {
        serde_json::from_value(v)
            .map_err(|e| JarvisError::invalid(format!("Argumentos no válidos: {e}")))
    }

    Ok(match name {
        SEARCH_NOTES => {
            let r: SearchRaw = de(value)?;
            ToolArgs::SearchNotes {
                query: text_field("query", &r.query, 1, 200)?,
                limit: int_field("limit", r.limit, 1, 10, 5)?,
            }
        }
        READ_NOTE => {
            let r: ReadRaw = de(value)?;
            check_relative(&r.path)?;
            ToolArgs::ReadNote {
                path: text_field("path", &r.path, 1, 400)?,
                query: opt_text("query", r.query, 200)?,
                max_chars: int_field("max_chars", r.max_chars, 200, 4000, 2000)?,
            }
        }
        LIST_TASKS => {
            let r: TasksRaw = de(value)?;
            ToolArgs::ListTasks(TaskFilterArgs {
                status: enum_field("status", r.status, &["open", "done", "all"], "open")?,
                query: opt_text("query", r.query, 200)?,
                project: opt_text("project", r.project, 120)?,
                due: enum_field("due", r.due, &["any", "today", "overdue", "week"], "any")?,
                limit: int_field("limit", r.limit, 1, 50, 20)?,
            })
        }
        LIST_PROJECTS => {
            let r: LimitRaw = de(value)?;
            ToolArgs::ListProjects {
                limit: int_field("limit", r.limit, 1, 30, 15)?,
            }
        }
        DAILY_SUMMARY => {
            let r: DailyRaw = de(value)?;
            ToolArgs::DailySummary {
                date: opt_date("date", r.date)?,
            }
        }
        CREATE_NOTE => {
            let r: CreateRaw = de(value)?;
            let folder = opt_text("folder", r.folder, 200)?;
            if let Some(f) = &folder {
                // La carpeta debe ser una ruta relativa segura.
                check_relative(&format!("{}/x.md", f.trim_matches('/')))?;
            }
            if r.content.chars().count() > 20_000 {
                return Err(JarvisError::invalid("content supera 20000 caracteres."));
            }
            ToolArgs::CreateNote {
                title: text_field("title", &r.title, 1, 120)?,
                content: r.content,
                folder,
            }
        }
        ADD_TASK => {
            let r: TaskRaw = de(value)?;
            let text = text_field("text", &r.text, 1, 300)?;
            if text.contains('\n') || text.contains('\r') {
                return Err(JarvisError::invalid("text debe ser una sola línea."));
            }
            ToolArgs::AddTask {
                text,
                due: opt_date("due", r.due)?,
            }
        }
        DELEGATE_TO_CLAUDE => {
            let r: DelegateRaw = de(value)?;
            let note_paths = r.note_paths.unwrap_or_default();
            if note_paths.len() > 3 {
                return Err(JarvisError::invalid(
                    "note_paths admite como máximo 3 notas.",
                ));
            }
            for p in &note_paths {
                check_relative(p)?;
            }
            ToolArgs::DelegateToClaude {
                task: text_field("task", &r.task, 1, 4000)?,
                context: opt_text("context", r.context, 8000)?,
                note_paths,
            }
        }
        _ => unreachable!("comprobado arriba"),
    })
}

impl ToolArgs {
    /// Descripción breve y legible de la operación, para el diálogo de
    /// confirmación. No incluye contenido completo.
    pub fn summary(&self) -> String {
        match self {
            ToolArgs::CreateNote {
                title,
                folder,
                content,
            } => format!(
                "Crear la nota «{title}» en {} ({} caracteres)",
                folder.as_deref().unwrap_or("la carpeta de JARVIS"),
                content.chars().count()
            ),
            ToolArgs::AddTask { text, due } => match due {
                Some(d) => format!("Añadir la tarea «{text}» con vencimiento {d}"),
                None => format!("Añadir la tarea «{text}»"),
            },
            ToolArgs::DelegateToClaude {
                task, note_paths, ..
            } => format!(
                "Enviar a Claude la tarea «{}»{}",
                crate::vault::truncate_chars(task, 120).0,
                if note_paths.is_empty() {
                    String::new()
                } else {
                    format!(" con fragmentos de {}", note_paths.join(", "))
                }
            ),
            ToolArgs::SearchNotes { query, .. } => format!("Buscar «{query}» en la Vault"),
            ToolArgs::ReadNote { path, .. } => format!("Leer la nota {path}"),
            ToolArgs::ListTasks(_) => "Consultar tareas".into(),
            ToolArgs::ListProjects { .. } => "Consultar proyectos".into(),
            ToolArgs::DailySummary { .. } => "Consultar el resumen del día".into(),
        }
    }

    /// Vista previa del contenido para el diálogo de confirmación.
    pub fn preview(&self) -> Option<String> {
        match self {
            ToolArgs::CreateNote { content, .. } => {
                Some(crate::vault::truncate_chars(content, 600).0)
            }
            ToolArgs::DelegateToClaude { context, .. } => context
                .as_ref()
                .map(|c| crate::vault::truncate_chars(c, 600).0),
            _ => None,
        }
    }
}

fn text_field(name: &str, v: &str, min: usize, max: usize) -> Result<String> {
    let t = v.trim();
    let n = t.chars().count();
    if n < min || n > max {
        return Err(JarvisError::invalid(format!(
            "{name} debe tener entre {min} y {max} caracteres."
        )));
    }
    Ok(t.to_string())
}

fn opt_text(name: &str, v: Option<String>, max: usize) -> Result<Option<String>> {
    match v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if s.chars().count() > max => Err(JarvisError::invalid(format!(
            "{name} supera {max} caracteres."
        ))),
        Some(s) => Ok(Some(s)),
    }
}

fn int_field(name: &str, v: Option<i64>, min: i64, max: i64, default: usize) -> Result<usize> {
    match v {
        None => Ok(default),
        Some(n) if (min..=max).contains(&n) => Ok(n as usize),
        Some(_) => Err(JarvisError::invalid(format!(
            "{name} debe estar entre {min} y {max}."
        ))),
    }
}

fn enum_field(
    name: &str,
    v: Option<String>,
    allowed: &[&'static str],
    default: &'static str,
) -> Result<&'static str> {
    match v {
        None => Ok(default),
        Some(s) => allowed.iter().copied().find(|a| *a == s).ok_or_else(|| {
            JarvisError::invalid(format!("{name} debe ser uno de: {}.", allowed.join(", ")))
        }),
    }
}

fn opt_date(name: &str, v: Option<String>) -> Result<Option<NaiveDate>> {
    match v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| JarvisError::invalid(format!("{name} debe tener formato AAAA-MM-DD."))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn accepts_valid_arguments_as_object_or_string() {
        let a = parse_args(SEARCH_NOTES, &json!({"query": "proyecto alfa"}), false).unwrap();
        assert_eq!(
            a,
            ToolArgs::SearchNotes {
                query: "proyecto alfa".into(),
                limit: 5
            }
        );
        let b = parse_args(
            ADD_TASK,
            &json!("{\"text\":\"Llamar\",\"due\":\"2026-10-12\"}"),
            false,
        )
        .unwrap();
        assert!(matches!(b, ToolArgs::AddTask { .. }));
    }

    #[test]
    fn rejects_invalid_arguments() {
        let cases = [
            (SEARCH_NOTES, json!({})),
            (SEARCH_NOTES, json!({"query": ""})),
            (SEARCH_NOTES, json!({"query": "x", "limit": 99})),
            (SEARCH_NOTES, json!({"query": "x", "rm": "-rf"})),
            (READ_NOTE, json!({"path": "../../etc/passwd"})),
            (READ_NOTE, json!({"path": 42})),
            (LIST_TASKS, json!({"status": "deleted"})),
            (ADD_TASK, json!({"text": "a\nb"})),
            (ADD_TASK, json!({"text": "ok", "due": "mañana"})),
            (CREATE_NOTE, json!({"title": "x"})),
            (
                CREATE_NOTE,
                json!({"title": "x", "content": "y", "folder": "../fuera"}),
            ),
            (DAILY_SUMMARY, json!("no es json")),
            (DAILY_SUMMARY, json!([1, 2])),
        ];
        for (name, args) in cases {
            let err = parse_args(name, &args, false).unwrap_err();
            assert!(
                matches!(
                    err.code,
                    ErrorCode::InvalidArguments | ErrorCode::PermissionDenied
                ),
                "{name} {args} -> {:?}",
                err.code
            );
        }
    }

    #[test]
    fn rejects_unknown_and_unavailable_tools() {
        assert_eq!(
            parse_args("run_shell", &json!({}), true).unwrap_err().code,
            ErrorCode::UnknownTool
        );
        assert_eq!(
            parse_args(DELEGATE_TO_CLAUDE, &json!({"task": "x"}), false)
                .unwrap_err()
                .code,
            ErrorCode::UnknownTool
        );
    }

    #[test]
    fn schemas_forbid_additional_properties() {
        for t in catalog(true) {
            assert_eq!(
                t.parameters["additionalProperties"],
                json!(false),
                "{}",
                t.name
            );
            assert_eq!(t.parameters["type"], "object");
        }
    }
}
