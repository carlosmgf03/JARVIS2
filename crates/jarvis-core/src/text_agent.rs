//! Interfaz de texto.
//!
//! Reutiliza exactamente el mismo ejecutor, catálogo, política y Vault que la
//! voz. Proveedores:
//! - **Claude** (si hay clave): conversación en lenguaje natural con
//!   herramientas.
//! - **Comandos**: modo determinista sin modelo (`/buscar`, `/tareas`, …).
//!   Funciona sin ninguna credencial y cuando Realtime no está disponible.
//!
//! El historial que se reenvía a Claude es compacto (solo texto final de cada
//! turno, sin resultados intermedios de herramientas) y acotado.

use std::sync::{Arc, Mutex};

use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};

use crate::anthropic::{text_of, ClaudeClient};
use crate::config::TextProvider;
use crate::error::{ErrorCode, JarvisError, Result};
use crate::executor::{Executor, Origin, OutcomeStatus, ToolCall, ToolOutcome};
use crate::tools;
use crate::vault::truncate_chars;

const MAX_HISTORY: usize = 16;
const MAX_TOOL_ROUNDS: usize = 6;

#[derive(Debug, Clone, Serialize)]
pub struct ToolTrace {
    pub tool: String,
    pub status: OutcomeStatus,
    pub code: Option<ErrorCode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextReply {
    pub reply: String,
    pub provider: &'static str,
    pub tools: Vec<ToolTrace>,
}

pub struct TextAgent {
    executor: Arc<Executor>,
    claude: Option<Arc<ClaudeClient>>,
    history: Mutex<Vec<(String, String)>>,
    user_name: Option<String>,
}

impl TextAgent {
    pub fn new(
        executor: Arc<Executor>,
        claude: Option<Arc<ClaudeClient>>,
        provider: TextProvider,
    ) -> Self {
        let claude = match provider {
            TextProvider::Commands => None,
            TextProvider::Auto | TextProvider::Claude => claude,
        };
        Self {
            executor,
            claude,
            history: Mutex::new(Vec::new()),
            user_name: None,
        }
    }

    /// Nombre del usuario para el trato personal (ver `persona::user_section`).
    pub fn with_user_name(mut self, name: Option<String>) -> Self {
        self.user_name = name;
        self
    }

    pub fn provider(&self) -> &'static str {
        if self.claude.is_some() {
            "claude"
        } else {
            "commands"
        }
    }

    pub fn reset(&self) {
        self.history.lock().expect("mutex").clear();
    }

    /// `recent_context`: últimos turnos de la conversación de voz, recortados
    /// por la interfaz, para que el cambio de canal no pierda el hilo.
    pub async fn send(&self, message: &str, recent_context: Option<&str>) -> Result<TextReply> {
        let message = message.trim();
        if message.is_empty() {
            return Err(JarvisError::invalid("El mensaje está vacío."));
        }
        if message.chars().count() > 8000 {
            return Err(JarvisError::invalid(
                "El mensaje es demasiado largo (máx. 8000 caracteres).",
            ));
        }
        if message.starts_with('/') || self.claude.is_none() {
            return Ok(self.run_command(message).await);
        }
        self.run_claude(message, recent_context).await
    }

    // ------------------------------------------------------------- Claude

    async fn run_claude(&self, message: &str, recent_context: Option<&str>) -> Result<TextReply> {
        let claude = self.claude.clone().expect("comprobado");
        let mut messages: Vec<Value> = self
            .history
            .lock()
            .expect("mutex")
            .iter()
            .map(|(role, text)| json!({"role": role, "content": text}))
            .collect();
        let mut user = String::new();
        if let Some(ctx) = recent_context.filter(|c| !c.trim().is_empty()) {
            user.push_str(&format!(
                "<conversacion_de_voz_reciente>\n{}\n</conversacion_de_voz_reciente>\n\n",
                truncate_chars(ctx, 2000).0
            ));
        }
        user.push_str(message);
        messages.push(json!({"role": "user", "content": user}));

        let tool_defs: Vec<Value> = tools::catalog(tools::Availability {
            claude: false,
            ..self.executor.availability()
        })
        .into_iter()
        .map(
            |t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters}),
        )
        .collect();
        let system = format!(
            "{}{}\nFecha y hora local: {}.",
            crate::persona::TEXT_INSTRUCTIONS,
            crate::persona::user_section(self.user_name.as_deref()),
            Local::now().format("%A %d/%m/%Y %H:%M")
        );

        let mut traces = Vec::new();
        for _ in 0..MAX_TOOL_ROUNDS {
            let resp = claude
                .messages(json!({
                    "max_tokens": 1500,
                    "system": system,
                    "tools": tool_defs,
                    "messages": messages,
                }))
                .await?;
            let content = resp.get("content").cloned().unwrap_or_else(|| json!([]));
            let tool_uses: Vec<Value> = content
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            if tool_uses.is_empty()
                || resp.get("stop_reason").and_then(Value::as_str) != Some("tool_use")
            {
                let reply = text_of(&resp);
                self.remember(message, &reply);
                return Ok(TextReply {
                    reply,
                    provider: "claude",
                    tools: traces,
                });
            }
            messages.push(json!({"role": "assistant", "content": content}));
            let mut results = Vec::new();
            for tu in tool_uses {
                let outcome = self
                    .executor
                    .invoke(ToolCall {
                        call_id: tu
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: tu
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: tu.get("input").cloned().unwrap_or(Value::Null),
                        origin: Origin::Text,
                    })
                    .await;
                traces.push(trace(&outcome));
                results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tu.get("id"),
                    "content": outcome.for_model().to_string(),
                    "is_error": outcome.status != OutcomeStatus::Ok,
                }));
            }
            messages.push(json!({"role": "user", "content": results}));
        }
        let reply =
            "He alcanzado el límite de pasos para esta petición. ¿Puedes concretarla un poco más?"
                .to_string();
        self.remember(message, &reply);
        Ok(TextReply {
            reply,
            provider: "claude",
            tools: traces,
        })
    }

    fn remember(&self, user: &str, assistant: &str) {
        let mut h = self.history.lock().expect("mutex");
        h.push(("user".into(), truncate_chars(user, 2000).0));
        h.push(("assistant".into(), truncate_chars(assistant, 2000).0));
        let excess = h.len().saturating_sub(MAX_HISTORY);
        h.drain(..excess);
    }

    // ----------------------------------------------------------- comandos

    async fn run_command(&self, message: &str) -> TextReply {
        let (cmd, rest) = match message.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (message, ""),
        };
        let call = |name: &str, args: Value| ToolCall {
            call_id: format!("txt-{}", uuid::Uuid::new_v4()),
            name: name.into(),
            arguments: args,
            origin: Origin::Text,
        };
        let request = match cmd.to_lowercase().as_str() {
            "/buscar" => Some(call(tools::SEARCH_NOTES, json!({"query": rest}))),
            "/leer" => {
                let (path, topic) = split_pipe(rest);
                let mut a = json!({"path": path});
                if let Some(t) = topic {
                    a["query"] = json!(t);
                }
                Some(call(tools::READ_NOTE, a))
            }
            "/tareas" => {
                let mut a = json!({});
                let mut words = rest.split_whitespace().peekable();
                match words.peek().map(|w| w.to_lowercase()) {
                    Some(w) if w == "hoy" => {
                        a["due"] = json!("today");
                        words.next();
                    }
                    Some(w) if w == "vencidas" || w == "atrasadas" => {
                        a["due"] = json!("overdue");
                        words.next();
                    }
                    Some(w) if w == "semana" => {
                        a["due"] = json!("week");
                        words.next();
                    }
                    Some(w) if w == "hechas" => {
                        a["status"] = json!("done");
                        words.next();
                    }
                    Some(w) if w == "todas" => {
                        a["status"] = json!("all");
                        words.next();
                    }
                    _ => {}
                }
                let q: Vec<&str> = words.collect();
                if !q.is_empty() {
                    a["query"] = json!(q.join(" "));
                }
                Some(call(tools::LIST_TASKS, a))
            }
            "/proyectos" => Some(call(tools::LIST_PROJECTS, json!({}))),
            "/clima" | "/tiempo" => Some(call(
                tools::GET_WEATHER,
                if rest.is_empty() {
                    json!({"days": 3})
                } else {
                    json!({"location": rest, "days": 3})
                },
            )),
            "/hora" => Some(call(
                tools::GET_TIME,
                if rest.is_empty() {
                    json!({})
                } else {
                    json!({"location": rest})
                },
            )),
            "/trafico" | "/tráfico" => Some(call(
                tools::GET_TRAFFIC,
                if rest.is_empty() {
                    json!({})
                } else {
                    json!({"destination": rest})
                },
            )),
            "/vuelos" => {
                // /vuelos MDE 2026-10-20 [2026-10-25]
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let mut a = json!({"destination": parts.first().copied().unwrap_or(""), "date": parts.get(1).copied().unwrap_or("")});
                if let Some(r) = parts.get(2) {
                    a["return_date"] = json!(r);
                }
                Some(call(tools::SEARCH_FLIGHTS, a))
            }
            "/web" => Some(call(tools::WEB_SEARCH, json!({"query": rest}))),
            "/pagina" | "/página" => Some(call(tools::FETCH_PAGE, json!({"url": rest}))),
            "/abrir" => Some(call(tools::OPEN_URL, json!({"url": rest}))),
            "/correos" => Some(call(
                tools::LIST_EMAILS,
                json!({"unread_only": rest.eq_ignore_ascii_case("no leídos") || rest.eq_ignore_ascii_case("no leidos") || rest.eq_ignore_ascii_case("nuevos")}),
            )),
            "/correo" => match rest.parse::<i64>() {
                Ok(id) => Some(call(tools::READ_EMAIL, json!({"id": id}))),
                Err(_) => Some(call(tools::READ_EMAIL, json!({"id": rest}))),
            },
            "/resumen" => Some(call(
                tools::DAILY_SUMMARY,
                if rest.is_empty() {
                    json!({})
                } else {
                    json!({"date": rest})
                },
            )),
            "/nota" => {
                let (title, content) = split_pipe(rest);
                Some(call(
                    tools::CREATE_NOTE,
                    json!({"title": title, "content": content.unwrap_or("")}),
                ))
            }
            "/tarea" => {
                let (text, due) = match rest.rsplit_once(" @") {
                    Some((t, d)) if d.len() == 10 => (t, Some(d)),
                    _ => (rest, None),
                };
                let mut a = json!({"text": text});
                if let Some(d) = due {
                    a["due"] = json!(d);
                }
                Some(call(tools::ADD_TASK, a))
            }
            _ => None,
        };
        let Some(request) = request else {
            let mut reply = String::new();
            if !message.starts_with('/') {
                reply.push_str("No hay un modelo de texto configurado (Claude), así que uso el modo de comandos.\n\n");
            }
            reply.push_str(COMMAND_HELP);
            return TextReply {
                reply,
                provider: "commands",
                tools: vec![],
            };
        };
        let outcome = self.executor.invoke(request).await;
        TextReply {
            reply: format_outcome(&outcome),
            provider: "commands",
            tools: vec![trace(&outcome)],
        }
    }
}

pub const COMMAND_HELP: &str = "Comandos disponibles:\n\
/buscar <texto> — buscar en la Vault\n\
/leer <ruta> [| tema] — leer una nota o sus fragmentos sobre un tema\n\
/tareas [hoy|vencidas|semana|hechas|todas] [texto] — consultar tareas\n\
/proyectos — listar proyectos\n\
/resumen [AAAA-MM-DD] — resumen del día\n\
/nota <título> | <contenido> — crear una nota (pide confirmación)\n\
/tarea <texto> [@AAAA-MM-DD] — añadir una tarea (pide confirmación)\n\
/clima [ciudad o país] — tiempo actual y previsión\n\
/hora [ciudad o país] — hora local en cualquier lugar\n\
/trafico [destino] — tráfico en Bogotá (y tiempo hasta el destino)\n\
/vuelos <IATA destino> <AAAA-MM-DD> [vuelta] — precios y horarios, con días cercanos\n\
/web <consulta> — buscar en Internet\n\
/pagina <dirección> — leer el texto de una página\n\
/abrir <dirección> — abrir una página en el navegador (pide confirmación)\n\
/correos [nuevos] — últimos correos de la bandeja de entrada\n\
/correo <id> — leer un correo";

fn split_pipe(s: &str) -> (&str, Option<&str>) {
    match s.split_once('|') {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (s.trim(), None),
    }
}

fn trace(o: &ToolOutcome) -> ToolTrace {
    ToolTrace {
        tool: o.tool.clone(),
        status: o.status,
        code: o.error.as_ref().map(|e| e.code),
    }
}

/// Presentación en texto plano de un resultado (modo comandos).
pub fn format_outcome(o: &ToolOutcome) -> String {
    if o.status != OutcomeStatus::Ok {
        let e = o.error.as_ref();
        let prefix = if o.status == OutcomeStatus::Denied {
            "No realizado"
        } else {
            "Error"
        };
        return format!(
            "{prefix}: {}",
            e.map(|e| e.message.as_str()).unwrap_or("desconocido")
        );
    }
    let d = o.data.clone().unwrap_or(Value::Null);
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match o.tool.as_str() {
        tools::SEARCH_NOTES => {
            let items = d["results"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return "Sin resultados.".into();
            }
            items
                .iter()
                .map(|r| {
                    let snip = r["snippets"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    format!("• {} ({})\n  {}", s(r, "title"), s(r, "path"), snip)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        tools::READ_NOTE => format!(
            "{} ({}){}\n\n{}",
            s(&d, "title"),
            s(&d, "path"),
            if d["truncated"].as_bool() == Some(true) {
                " — extracto"
            } else {
                ""
            },
            s(&d, "content")
        ),
        tools::LIST_TASKS => format_tasks(&d["tasks"], "No hay tareas que coincidan."),
        tools::LIST_PROJECTS => {
            let items = d["projects"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return "No se encontraron proyectos.".into();
            }
            items
                .iter()
                .map(|p| {
                    format!(
                        "• {} — {} tareas abiertas ({})",
                        s(p, "name"),
                        p["open_tasks"],
                        s(p, "path")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        tools::DAILY_SUMMARY => {
            let mut out = format!("Resumen del {}\n", s(&d, "date"));
            out.push_str(&format!(
                "Nota diaria: {}\n",
                d["daily_note"]
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("no existe")
            ));
            out.push_str(&format!(
                "Tareas abiertas en total: {}\n\n",
                d["open_tasks_total"]
            ));
            out.push_str("Vencen hoy:\n");
            out.push_str(&format_tasks(&d["due_today"], "  (ninguna)"));
            out.push_str("\n\nAtrasadas:\n");
            out.push_str(&format_tasks(&d["overdue"], "  (ninguna)"));
            out
        }
        tools::CREATE_NOTE => format!("Nota creada y verificada: {}", s(&d, "path")),
        tools::ADD_TASK => format!(
            "Tarea añadida y verificada en {} (línea {}): {}",
            s(&d, "path"),
            d["line"],
            s(&d, "text")
        ),
        tools::GET_WEATHER => {
            let n = &d["now"];
            let mut out = format!(
                "{}: {} °C ({}), sensación {} °C, humedad {} %, viento {} km/h",
                s(&d, "location"),
                n["temperature_c"],
                s(n, "description"),
                n["feels_like_c"],
                n["humidity_pct"],
                n["wind_kmh"]
            );
            for day in d["days"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "\n• {}: {}, {}–{} °C, lluvia {} %",
                    s(day, "date"),
                    s(day, "description"),
                    day["min_c"],
                    day["max_c"],
                    day["rain_probability_pct"]
                ));
            }
            out.push_str("\nFuente: Open-Meteo");
            out
        }
        tools::GET_TIME => format!(
            "{}: {} — {} (UTC{}){}",
            s(&d, "location"),
            s(&d, "time"),
            s(&d, "date"),
            s(&d, "utc_offset"),
            if d["multiple_timezones"].as_bool() == Some(true) {
                "\nEste país tiene varias zonas horarias; es la hora de la capital."
            } else {
                ""
            }
        ),
        tools::GET_TRAFFIC => {
            let mut out = format!("Tráfico en {}", s(&d, "area"));
            if let Some(r) = d["route"].as_object() {
                out.push_str(&format!(
                    "\nDe {} a {}: {} min con tráfico ({} min de retraso), {} km",
                    r["from"].as_str().unwrap_or(""),
                    r["to"].as_str().unwrap_or(""),
                    r["minutes_with_traffic"],
                    r["delay_minutes"],
                    r["distance_km"]
                ));
            }
            let inc = d["incidents"].as_array().cloned().unwrap_or_default();
            if inc.is_empty() {
                out.push_str("\nSin incidentes destacados ahora mismo.");
            }
            for i in inc {
                out.push_str(&format!(
                    "\n• {} ({}): {} — {}",
                    s(&i, "kind"),
                    s(&i, "severity"),
                    s(&i, "road"),
                    s(&i, "description")
                ));
            }
            out.push_str("\nFuente: TomTom");
            out
        }
        tools::SEARCH_FLIGHTS => {
            let cur = s(&d, "currency");
            let fmt = |o: &Value| {
                format!(
                    "{} {} — {} · sale {} · llega {} · {} escala(s)",
                    o["price"],
                    cur,
                    s(o, "airlines"),
                    s(o, "departure"),
                    s(o, "arrival"),
                    o["stops"]
                )
            };
            let mut out = format!(
                "Vuelos {} ({}), {}",
                s(&d, "route"),
                s(&d, "trip"),
                s(&d, "requested_date")
            );
            for o in d["options"].as_array().into_iter().flatten() {
                out.push_str(&format!("\n• {}", fmt(o)));
            }
            out.push_str("\n\nMás barato por día:");
            for day in d["by_date"].as_array().into_iter().flatten() {
                match day["cheapest"].as_object() {
                    Some(_) => out.push_str(&format!(
                        "\n• {}: {} {} ({})",
                        s(day, "date"),
                        day["cheapest"]["price"],
                        cur,
                        s(&day["cheapest"], "airlines")
                    )),
                    None => out.push_str(&format!("\n• {}: sin resultados", s(day, "date"))),
                }
            }
            if let Some(c) = d["cheapest_overall"].as_object() {
                out.push_str(&format!(
                    "\n\nMás económico: {} — {}",
                    c["date"].as_str().unwrap_or(""),
                    fmt(&d["cheapest_overall"])
                ));
            }
            out.push_str(&format!("\n{}", s(&d, "note")));
            out
        }
        tools::WEB_SEARCH => {
            let mut out = s(&d, "answer");
            for src in d["sources"].as_array().into_iter().flatten() {
                out.push_str(&format!("\n• {} — {}", s(src, "title"), s(src, "url")));
            }
            out
        }
        tools::FETCH_PAGE => format!(
            "{} ({}){}\n\n{}",
            d["title"].as_str().unwrap_or("Sin título"),
            s(&d, "url"),
            if d["truncated"].as_bool() == Some(true) {
                " — extracto"
            } else {
                ""
            },
            s(&d, "content")
        ),
        tools::OPEN_URL => format!("Abierta en el navegador: {}", s(&d, "opened")),
        tools::LIST_EMAILS => {
            let items = d["emails"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return "No hay correos que coincidan.".into();
            }
            items
                .iter()
                .map(|m| {
                    format!(
                        "{} [{}] {} — {}",
                        if m["unread"].as_bool() == Some(true) {
                            "●"
                        } else {
                            "○"
                        },
                        m["id"],
                        s(m, "from"),
                        s(m, "subject")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        tools::READ_EMAIL => format!(
            "De: {}\nAsunto: {}\nFecha: {}\n\n{}",
            s(&d, "from"),
            s(&d, "subject"),
            s(&d, "date"),
            s(&d, "body")
        ),
        _ => serde_json::to_string_pretty(&d).unwrap_or_default(),
    }
}

fn format_tasks(v: &Value, empty: &str) -> String {
    let items = v.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return empty.into();
    }
    items
        .iter()
        .map(|t| {
            format!(
                "{} {} — {}:{}",
                if t["done"].as_bool() == Some(true) {
                    "☑"
                } else {
                    "☐"
                },
                t["text"].as_str().unwrap_or_default(),
                t["path"].as_str().unwrap_or_default(),
                t["line"]
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
