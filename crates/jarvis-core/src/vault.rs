//! Acceso a la Vault de Obsidian.
//!
//! Todas las rutas que llegan del modelo son relativas a la raíz de la Vault y
//! se validan antes de tocar el disco: sin rutas absolutas, sin `..`, sin
//! carpetas ocultas (`.obsidian`, `.git`, `.trash`) y, tras resolver enlaces
//! simbólicos, siempre dentro de la raíz. Solo se leen y crean ficheros `.md`.
//! Las escrituras nunca sobrescriben ni borran: crear nota usa `create_new` y
//! añadir tarea solo agrega una línea al final. Toda escritura se verifica
//! releyendo el fichero antes de declararse correcta.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use regex::Regex;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::config::VaultConfig;
use crate::error::{ErrorCode, JarvisError, Result};

const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_FILES: usize = 20_000;
const SNIPPET_CHARS: usize = 200;

pub struct Vault {
    root: PathBuf,
    cfg: VaultConfig,
    write_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub score: u32,
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteExcerpt {
    pub path: String,
    pub title: String,
    pub content: String,
    pub truncated: bool,
    pub total_chars: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Task {
    pub text: String,
    pub done: bool,
    pub path: String,
    pub line: usize,
    pub due: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Open,
    Done,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueFilter {
    Any,
    Today,
    Overdue,
    Week,
}

#[derive(Debug, Clone)]
pub struct TaskFilter {
    pub status: TaskStatus,
    pub query: Option<String>,
    pub project: Option<String>,
    pub due: DueFilter,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub status: Option<String>,
    pub open_tasks: usize,
    pub done_tasks: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub path: String,
    pub line: Option<usize>,
    pub text: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailySummary {
    pub date: String,
    pub daily_note: Option<NoteExcerpt>,
    pub due_today: Vec<Task>,
    pub overdue: Vec<Task>,
    pub open_tasks_total: usize,
    pub modified_notes: Vec<String>,
}

impl Vault {
    pub fn open(cfg: &VaultConfig) -> Result<Self> {
        let path = cfg.path.as_ref().ok_or_else(|| {
            JarvisError::new(
                ErrorCode::VaultNotConfigured,
                "No hay Vault configurada. Define JARVIS_VAULT_PATH.",
            )
        })?;
        let root = fs::canonicalize(path).map_err(|e| {
            JarvisError::new(
                ErrorCode::VaultNotConfigured,
                format!("La Vault «{}» no es accesible: {e}", path.display()),
            )
        })?;
        if !root.is_dir() {
            return Err(JarvisError::new(
                ErrorCode::VaultNotConfigured,
                "JARVIS_VAULT_PATH debe ser una carpeta.",
            ));
        }
        Ok(Self {
            root,
            cfg: cfg.clone(),
            write_lock: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ---------------------------------------------------------------- lectura

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return Err(JarvisError::invalid(
                "La búsqueda no contiene términos útiles.",
            ));
        }
        let mut hits: Vec<(SearchHit, SystemTime)> = Vec::new();
        for file in self.markdown_files()? {
            let Ok(body) = fs::read_to_string(&file) else {
                continue;
            };
            let title = stem(&file);
            let n_title = normalize(&title);
            let n_body = normalize(&body);
            let mut score = 0u32;
            let mut matched = 0usize;
            for t in &terms {
                let in_title = n_title.contains(t.as_str());
                let count = n_body.matches(t.as_str()).count().min(10) as u32;
                if in_title || count > 0 {
                    matched += 1;
                }
                score += if in_title { 3 } else { 0 } + count;
            }
            if matched == 0 {
                continue;
            }
            if matched == terms.len() && terms.len() > 1 {
                score += 5;
            }
            let snippets = body
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter(|l| {
                    let nl = normalize(l);
                    terms.iter().any(|t| nl.contains(t.as_str()))
                })
                .take(3)
                .map(|l| truncate_chars(l.trim(), SNIPPET_CHARS).0)
                .collect();
            let mtime = fs::metadata(&file)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            hits.push((
                SearchHit {
                    path: self.rel(&file),
                    title,
                    score,
                    snippets,
                },
                mtime,
            ));
        }
        hits.sort_by(|a, b| b.0.score.cmp(&a.0.score).then(b.1.cmp(&a.1)));
        Ok(hits.into_iter().take(limit).map(|(h, _)| h).collect())
    }

    pub fn read(&self, rel: &str, query: Option<&str>, max_chars: usize) -> Result<NoteExcerpt> {
        let abs = self.resolve_existing(rel)?;
        let body = read_note_file(&abs)?;
        let total_chars = body.chars().count();
        let content = match query.map(search_terms).filter(|t| !t.is_empty()) {
            Some(terms) => relevant_fragments(&body, &terms, max_chars),
            None => truncate_chars(&body, max_chars).0,
        };
        let truncated = content.chars().count() < total_chars;
        Ok(NoteExcerpt {
            path: self.rel(&abs),
            title: stem(&abs),
            content,
            truncated,
            total_chars,
        })
    }

    pub fn tasks(&self, filter: &TaskFilter, today: NaiveDate) -> Result<Vec<Task>> {
        let query = filter.query.as_deref().map(normalize);
        let project = filter.project.as_deref().map(normalize);
        let mut out = Vec::new();
        for file in self.markdown_files()? {
            let Ok(body) = fs::read_to_string(&file) else {
                continue;
            };
            let rel = self.rel(&file);
            let n_rel = normalize(&rel);
            for task in parse_tasks(&body, &rel) {
                let keep_status = match filter.status {
                    TaskStatus::Open => !task.done,
                    TaskStatus::Done => task.done,
                    TaskStatus::All => true,
                };
                if !keep_status || !due_matches(filter.due, task.due.as_deref(), today) {
                    continue;
                }
                let n_text = normalize(&task.text);
                if let Some(q) = &query {
                    if !n_text.contains(q.as_str()) {
                        continue;
                    }
                }
                if let Some(p) = &project {
                    if !n_rel.contains(p.as_str()) && !n_text.contains(&format!("#{p}")) {
                        continue;
                    }
                }
                out.push(task);
            }
        }
        out.sort_by(|a, b| match (&a.due, &b.due) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path).then(a.line.cmp(&b.line)),
        });
        out.truncate(filter.limit);
        Ok(out)
    }

    pub fn projects(&self, limit: usize) -> Result<Vec<Project>> {
        let folder = normalize(&format!("{}/", self.cfg.projects_folder.trim_matches('/')));
        let mut out = Vec::new();
        for file in self.markdown_files()? {
            let rel = self.rel(&file);
            let Ok(body) = fs::read_to_string(&file) else {
                continue;
            };
            let fm = frontmatter_value(&body, "type").map(|v| normalize(&v));
            let in_folder = normalize(&rel).starts_with(&folder);
            let typed = matches!(fm.as_deref(), Some("project" | "proyecto"));
            if !in_folder && !typed {
                continue;
            }
            let tasks = parse_tasks(&body, &rel);
            out.push(Project {
                name: stem(&file),
                path: rel,
                status: frontmatter_value(&body, "status")
                    .or_else(|| frontmatter_value(&body, "estado")),
                open_tasks: tasks.iter().filter(|t| !t.done).count(),
                done_tasks: tasks.iter().filter(|t| t.done).count(),
            });
        }
        out.sort_by(|a, b| b.open_tasks.cmp(&a.open_tasks).then(a.name.cmp(&b.name)));
        out.truncate(limit);
        Ok(out)
    }

    pub fn daily_summary(&self, date: NaiveDate) -> Result<DailySummary> {
        let daily_rel = format!(
            "{}/{}.md",
            self.cfg.daily_folder.trim_matches('/'),
            date.format(&self.cfg.daily_format)
        );
        let daily_note = match self.read(&daily_rel, None, 1500) {
            Ok(n) => Some(n),
            Err(e) if e.code == ErrorCode::NotFound => None,
            Err(e) => return Err(e),
        };
        let all_open = self.tasks(
            &TaskFilter {
                status: TaskStatus::Open,
                query: None,
                project: None,
                due: DueFilter::Any,
                limit: usize::MAX,
            },
            date,
        )?;
        let day = date.format("%Y-%m-%d").to_string();
        let due_today: Vec<Task> = all_open
            .iter()
            .filter(|t| t.due.as_deref() == Some(day.as_str()))
            .take(20)
            .cloned()
            .collect();
        let overdue: Vec<Task> = all_open
            .iter()
            .filter(|t| t.due.as_deref().is_some_and(|d| d < day.as_str()))
            .take(20)
            .cloned()
            .collect();
        let mut modified: Vec<(String, SystemTime)> = Vec::new();
        for file in self.markdown_files()? {
            if let Ok(m) = fs::metadata(&file).and_then(|m| m.modified()) {
                let local: DateTime<Local> = m.into();
                if local.date_naive() == date {
                    modified.push((self.rel(&file), m));
                }
            }
        }
        modified.sort_by_key(|m| std::cmp::Reverse(m.1));
        Ok(DailySummary {
            date: day,
            daily_note,
            due_today,
            overdue,
            open_tasks_total: all_open.len(),
            modified_notes: modified.into_iter().take(10).map(|(p, _)| p).collect(),
        })
    }

    // -------------------------------------------------------------- escritura

    pub fn create_note(
        &self,
        title: &str,
        content: &str,
        folder: Option<&str>,
        now: DateTime<Local>,
    ) -> Result<WriteResult> {
        let file_name = sanitize_title(title)?;
        let folder = folder.unwrap_or(&self.cfg.notes_folder);
        let rel = if folder.trim().is_empty() {
            format!("{file_name}.md")
        } else {
            format!("{}/{file_name}.md", folder.trim().trim_matches('/'))
        };
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| JarvisError::internal("bloqueo envenenado"))?;
        let abs = self.resolve_new(&rel)?;
        let body = format!(
            "---\ncreated: {}\nsource: jarvis\n---\n# {}\n\n{}\n",
            now.format("%Y-%m-%dT%H:%M:%S%:z"),
            title.trim(),
            content.trim_end()
        );
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&abs)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => JarvisError::new(
                    ErrorCode::AlreadyExists,
                    format!("Ya existe una nota llamada «{rel}». No se sobrescribe."),
                ),
                _ => io_err("crear la nota", e),
            })?;
        f.write_all(body.as_bytes())
            .map_err(|e| io_err("escribir la nota", e))?;
        f.sync_all().map_err(|e| io_err("guardar la nota", e))?;
        drop(f);
        let verified = fs::read_to_string(&abs).map(|b| b == body).unwrap_or(false);
        if !verified {
            return Err(JarvisError::new(
                ErrorCode::VerificationFailed,
                "La nota se escribió pero no se pudo verificar su contenido.",
            ));
        }
        Ok(WriteResult {
            path: self.rel(&abs),
            line: None,
            text: title.trim().to_string(),
            verified,
        })
    }

    pub fn add_task(&self, text: &str, due: Option<NaiveDate>) -> Result<WriteResult> {
        let text = text.trim().trim_start_matches("- [ ]").trim();
        let mut line = format!("- [ ] {text}");
        if let Some(d) = due {
            line.push_str(&format!(" 📅 {}", d.format("%Y-%m-%d")));
        }
        let rel = self.cfg.tasks_file.clone();
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| JarvisError::internal("bloqueo envenenado"))?;
        let abs = match self.resolve_existing(&rel) {
            Ok(p) => p,
            Err(e) if e.code == ErrorCode::NotFound => {
                let p = self.resolve_new(&rel)?;
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&p)
                    .map_err(|e| io_err("crear el fichero de tareas", e))?;
                f.write_all(b"# Tareas\n\n")
                    .map_err(|e| io_err("crear el fichero de tareas", e))?;
                p
            }
            Err(e) => return Err(e),
        };
        let existing =
            fs::read_to_string(&abs).map_err(|e| io_err("leer el fichero de tareas", e))?;
        let mut f = OpenOptions::new()
            .append(true)
            .open(&abs)
            .map_err(|e| io_err("abrir el fichero de tareas", e))?;
        let prefix = if existing.is_empty() || existing.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        f.write_all(format!("{prefix}{line}\n").as_bytes())
            .map_err(|e| io_err("añadir la tarea", e))?;
        f.sync_all().map_err(|e| io_err("guardar la tarea", e))?;
        drop(f);

        let after = fs::read_to_string(&abs).map_err(|e| io_err("verificar la tarea", e))?;
        let line_no = after
            .lines()
            .enumerate()
            .filter(|(_, l)| *l == line)
            .last()
            .map(|(i, _)| i + 1);
        let verified = line_no.is_some() && after.starts_with(&existing);
        if !verified {
            return Err(JarvisError::new(
                ErrorCode::VerificationFailed,
                "No se pudo verificar que la tarea quedara guardada.",
            ));
        }
        Ok(WriteResult {
            path: self.rel(&abs),
            line: line_no,
            text: line,
            verified,
        })
    }

    // ------------------------------------------------------------------ rutas

    fn rel(&self, abs: &Path) -> String {
        abs.strip_prefix(&self.root)
            .unwrap_or(abs)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn resolve_existing(&self, rel: &str) -> Result<PathBuf> {
        let rel = check_relative(rel)?;
        let joined = self.root.join(&rel);
        let abs = fs::canonicalize(&joined).map_err(|_| {
            JarvisError::new(
                ErrorCode::NotFound,
                format!("No existe la nota «{}».", rel.display()),
            )
        })?;
        if !abs.starts_with(&self.root) {
            return Err(outside_vault());
        }
        if !abs.is_file() {
            return Err(JarvisError::new(
                ErrorCode::NotFound,
                "La ruta no es una nota.",
            ));
        }
        Ok(abs)
    }

    fn resolve_new(&self, rel: &str) -> Result<PathBuf> {
        let rel = check_relative(rel)?;
        let file_name = rel
            .file_name()
            .ok_or_else(|| JarvisError::invalid("Ruta sin nombre de fichero."))?
            .to_owned();
        let parent_rel = rel.parent().unwrap_or(Path::new(""));
        // Antes de crear carpetas, comprobamos que el ancestro existente más
        // cercano está dentro de la Vault (evita escapar mediante symlinks).
        let mut probe = self.root.join(parent_rel);
        while !probe.exists() {
            probe = probe
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone());
        }
        let probe = fs::canonicalize(&probe).map_err(|e| io_err("resolver la carpeta", e))?;
        if !probe.starts_with(&self.root) {
            return Err(outside_vault());
        }
        let parent = self.root.join(parent_rel);
        fs::create_dir_all(&parent).map_err(|e| io_err("crear la carpeta", e))?;
        let parent = fs::canonicalize(&parent).map_err(|e| io_err("resolver la carpeta", e))?;
        if !parent.starts_with(&self.root) {
            return Err(outside_vault());
        }
        Ok(parent.join(file_name))
    }

    fn markdown_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let walker = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
        for entry in walker.flatten() {
            if out.len() >= MAX_FILES {
                break;
            }
            let p = entry.path();
            if entry.file_type().is_file()
                && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
                && entry
                    .metadata()
                    .map(|m| m.len() <= MAX_FILE_BYTES)
                    .unwrap_or(false)
            {
                out.push(p.to_path_buf());
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ utilidades

/// Valida una ruta relativa proporcionada por el modelo o el usuario.
pub fn check_relative(rel: &str) -> Result<PathBuf> {
    let rel = rel.trim().replace('\\', "/");
    if rel.is_empty() || rel.len() > 400 {
        return Err(JarvisError::invalid("Ruta vacía o demasiado larga."));
    }
    if rel.chars().any(|c| c.is_control()) {
        return Err(JarvisError::invalid(
            "La ruta contiene caracteres de control.",
        ));
    }
    if rel.starts_with('/') || rel.chars().nth(1) == Some(':') {
        return Err(JarvisError::new(
            ErrorCode::PermissionDenied,
            "Solo se admiten rutas relativas a la Vault.",
        ));
    }
    let mut out = PathBuf::new();
    for comp in Path::new(&rel).components() {
        match comp {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.starts_with('.') {
                    return Err(JarvisError::new(
                        ErrorCode::PermissionDenied,
                        "No se permite acceder a ficheros o carpetas ocultas.",
                    ));
                }
                out.push(s.as_ref());
            }
            Component::CurDir => {}
            _ => return Err(outside_vault()),
        }
    }
    match out
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
    {
        Some(e) if e == "md" => {}
        Some(_) => {
            return Err(JarvisError::new(
                ErrorCode::PermissionDenied,
                "Solo se pueden leer o crear notas Markdown (.md).",
            ))
        }
        None => {
            out.set_extension("md");
        }
    }
    Ok(out)
}

fn outside_vault() -> JarvisError {
    JarvisError::new(
        ErrorCode::PermissionDenied,
        "La ruta queda fuera de la Vault.",
    )
}

fn io_err(action: &str, e: std::io::Error) -> JarvisError {
    JarvisError::new(
        ErrorCode::VaultIo,
        format!("No se pudo {action} en la Vault: {e}"),
    )
}

fn read_note_file(abs: &Path) -> Result<String> {
    let meta = fs::metadata(abs).map_err(|e| io_err("leer la nota", e))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(JarvisError::new(
            ErrorCode::VaultIo,
            "La nota es demasiado grande para leerla.",
        ));
    }
    fs::read_to_string(abs).map_err(|e| io_err("leer la nota", e))
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Minúsculas y sin tildes, para que «reunion» encuentre «Reunión».
pub fn normalize(s: &str) -> String {
    s.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

const STOPWORDS: &[&str] = &[
    "de", "la", "el", "en", "y", "a", "los", "las", "del", "un", "una", "que", "por", "con",
    "para", "mi", "mis", "me", "sobre", "al", "lo", "se", "es", "nota", "notas",
];

pub fn search_terms(query: &str) -> Vec<String> {
    let all: Vec<String> = normalize(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_string)
        .collect();
    let useful: Vec<String> = all
        .iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    let mut terms = if useful.is_empty() { all } else { useful };
    terms.dedup();
    terms.truncate(8);
    terms
}

pub fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    (out, true)
}

fn relevant_fragments(body: &str, terms: &[String], max_chars: usize) -> String {
    let mut scored: Vec<(usize, usize, &str)> = body
        .split("\n\n")
        .enumerate()
        .filter(|(_, p)| !p.trim().is_empty())
        .map(|(i, p)| {
            let np = normalize(p);
            (
                terms.iter().map(|t| np.matches(t.as_str()).count()).sum(),
                i,
                p.trim(),
            )
        })
        .filter(|(s, _, _)| *s > 0)
        .collect();
    if scored.is_empty() {
        return truncate_chars(body, max_chars).0;
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut chosen: Vec<(usize, &str)> = Vec::new();
    let mut used = 0;
    for (_, i, p) in scored {
        let len = p.chars().count();
        if used + len > max_chars && !chosen.is_empty() {
            break;
        }
        chosen.push((i, p));
        used += len + 3;
    }
    chosen.sort_by_key(|(i, _)| *i);
    let joined = chosen
        .into_iter()
        .map(|(_, p)| p)
        .collect::<Vec<_>>()
        .join("\n…\n");
    truncate_chars(&joined, max_chars).0
}

fn task_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*[-*+] \[([ xX])\] (.+)$").expect("regex válida"))
}

fn due_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:📅|due::?|vence:)\s*(\d{4}-\d{2}-\d{2})").expect("regex válida")
    })
}

pub fn parse_tasks(body: &str, rel: &str) -> Vec<Task> {
    body.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let c = task_re().captures(l)?;
            let text = c[2].trim().to_string();
            let due = due_re().captures(&text).map(|d| d[1].to_string());
            Some(Task {
                done: &c[1] != " ",
                text,
                path: rel.to_string(),
                line: i + 1,
                due,
            })
        })
        .collect()
}

fn due_matches(filter: DueFilter, due: Option<&str>, today: NaiveDate) -> bool {
    let parsed = due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    match (filter, parsed) {
        (DueFilter::Any, _) => true,
        (_, None) => false,
        (DueFilter::Today, Some(d)) => d == today,
        (DueFilter::Overdue, Some(d)) => d < today,
        (DueFilter::Week, Some(d)) => d >= today && d <= today + chrono::Days::new(7),
    }
}

fn frontmatter_value(body: &str, key: &str) -> Option<String> {
    let rest = body
        .strip_prefix("---\n")
        .or_else(|| body.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    rest[..end].lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// Convierte un título en un nombre de fichero seguro para Obsidian.
pub fn sanitize_title(title: &str) -> Result<String> {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return Err(JarvisError::invalid("El título de la nota no es válido."));
    }
    Ok(truncate_chars(trimmed, 120)
        .0
        .trim_end_matches('…')
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths() {
        for bad in [
            "../secreto.md",
            "/etc/passwd",
            "C:/x.md",
            ".obsidian/app.json",
            "a/../../b.md",
            "foto.png",
        ] {
            assert!(check_relative(bad).is_err(), "{bad} debería rechazarse");
        }
        assert_eq!(
            check_relative("Proyectos/Alfa").unwrap(),
            PathBuf::from("Proyectos/Alfa.md")
        );
    }

    #[test]
    fn normalizes_accents() {
        assert_eq!(normalize("Reunión CAFÉ"), "reunion cafe");
        assert_eq!(
            search_terms("notas de la reunión"),
            vec!["reunion".to_string()]
        );
    }

    #[test]
    fn parses_tasks_with_due_dates() {
        let t = parse_tasks(
            "- [ ] Llamar 📅 2026-10-11\n- [x] Hecho\n  * [ ] sub due:2026-01-02",
            "a.md",
        );
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].due.as_deref(), Some("2026-10-11"));
        assert!(t[1].done);
        assert_eq!(t[2].due.as_deref(), Some("2026-01-02"));
        assert_eq!(t[2].line, 3);
    }

    #[test]
    fn sanitizes_titles() {
        assert_eq!(
            sanitize_title("Plan: Q4 / ideas?").unwrap(),
            "Plan Q4 ideas"
        );
        assert!(sanitize_title("../..").is_err());
    }
}
