use chrono::{DateTime, Datelike, Local, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tauri::Manager;
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const ARCHIVE_SCHEMA: &str = "codex-session-transfer/v1";
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    id: String,
    title: String,
    created_at: String,
    updated_at: String,
    cwd: String,
    #[serde(default)]
    project_key: String,
    #[serde(default)]
    project_name: String,
    #[serde(default)]
    project_path: String,
    source: String,
    model_provider: String,
    git_branch: String,
    git_origin_url: String,
    first_user_message: String,
    preview: String,
    message_count: usize,
    user_message_count: usize,
    size: u64,
    archived: bool,
    #[serde(default = "default_project_exists")]
    project_exists: bool,
    #[serde(default = "default_codex_visible")]
    codex_visible: bool,
    #[serde(default)]
    project_pinned: bool,
    #[serde(skip_serializing, default)]
    file_path: PathBuf,
    #[serde(skip_serializing, default)]
    browser_file: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveTask {
    #[serde(flatten)]
    task: Task,
    session_file: String,
    browser_file: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema: String,
    created_at: String,
    source_platform: String,
    tasks: Vec<ArchiveTask>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskList {
    tasks: Vec<Task>,
    codex_home: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveInspection {
    canceled: bool,
    path: String,
    created_at: String,
    tasks: Vec<InspectedTask>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectedTask {
    #[serde(flatten)]
    task: Task,
    conflict: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportOptions {
    adapt_paths: Option<bool>,
    restore_existing: Option<bool>,
    target_cwd: Option<String>,
}

#[derive(Clone, Debug)]
struct DesktopProject {
    id: String,
    name: String,
    root_paths: Vec<String>,
    pinned: bool,
}

#[derive(Clone, Debug)]
struct ThreadProjectAssignment {
    project_id: String,
    cwd: String,
}

#[derive(Clone, Debug, Default)]
struct DesktopProjectState {
    projects: HashMap<String, DesktopProject>,
    assignments: HashMap<String, ThreadProjectAssignment>,
    client_threads: HashSet<String>,
}

fn default_project_exists() -> bool {
    true
}

fn default_codex_visible() -> bool {
    true
}

fn codex_home() -> PathBuf {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
}

fn codex_desktop_processes() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
            .ok();
        let text = output
            .as_ref()
            .map(|item| String::from_utf8_lossy(&item.stdout).to_string())
            .unwrap_or_default();
        return text
            .lines()
            .filter_map(|line| line.split(',').next())
            .map(|name| name.trim().trim_matches('"').to_string())
            .filter(|name| matches!(name.as_str(), "ChatGPT.exe" | "Codex.exe"))
            .collect();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid=,args="])
            .output()
            .ok();
        let text = output
            .as_ref()
            .map(|item| String::from_utf8_lossy(&item.stdout).to_string())
            .unwrap_or_default();
        text.lines()
            .filter(|line| {
                line.contains("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT")
                    || line.contains("/Applications/Codex.app/Contents/MacOS/Codex")
            })
            .map(|line| clean_text(line))
            .collect()
    }
}

fn is_codex_desktop_running() -> bool {
    !codex_desktop_processes().is_empty()
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(value: &str, limit: usize) -> String {
    let value = clean_text(value);
    if value.chars().count() <= limit {
        return value;
    }
    format!(
        "{}…",
        value
            .chars()
            .take(limit.saturating_sub(1))
            .collect::<String>()
    )
}

fn value_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.to_string(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("input_text"))
                    .or_else(|| part.get("output_text"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn session_details(path: &Path) -> Result<Task, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut task = Task {
        id: String::new(),
        title: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
        cwd: String::new(),
        project_key: String::new(),
        project_name: String::new(),
        project_path: String::new(),
        source: String::new(),
        model_provider: String::new(),
        git_branch: String::new(),
        git_origin_url: String::new(),
        first_user_message: String::new(),
        preview: String::new(),
        message_count: 0,
        user_message_count: 0,
        size: fs::metadata(path).map_err(|error| error.to_string())?.len(),
        archived: false,
        project_exists: true,
        codex_visible: true,
        project_pinned: false,
        file_path: path.to_path_buf(),
        browser_file: PathBuf::new(),
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = record.get("payload").unwrap_or(&Value::Null);
            if task.id.is_empty() {
                task.id = value_string(payload.get("id"));
            }
            if task.created_at.is_empty() {
                task.created_at =
                    value_string(payload.get("timestamp").or_else(|| record.get("timestamp")));
            }
            if task.cwd.is_empty() {
                task.cwd = value_string(payload.get("cwd"));
            }
            if task.source.is_empty() {
                task.source = value_string(payload.get("source"));
            }
            if task.model_provider.is_empty() {
                task.model_provider = value_string(payload.get("model_provider"));
            }
            if task.git_branch.is_empty() {
                task.git_branch =
                    value_string(payload.get("git").and_then(|git| git.get("branch")));
            }
            if task.git_origin_url.is_empty() {
                task.git_origin_url =
                    value_string(payload.get("git").and_then(|git| git.get("repository_url")));
            }
        }
        let payload = record.get("payload").unwrap_or(&Value::Null);
        let role = payload.get("role").and_then(Value::as_str);
        let text = if payload.get("type").and_then(Value::as_str) == Some("user_message") {
            value_string(payload.get("message").or_else(|| payload.get("text")))
        } else if payload.get("type").and_then(Value::as_str) == Some("message")
            && matches!(role, Some("user") | Some("assistant"))
        {
            content_text(payload.get("content"))
        } else {
            String::new()
        };
        if clean_text(&text).is_empty() {
            continue;
        }
        task.message_count += 1;
        if role == Some("user")
            || payload.get("type").and_then(Value::as_str) == Some("user_message")
        {
            task.user_message_count += 1;
            if task.first_user_message.is_empty() {
                task.first_user_message = clean_text(&text);
            }
            task.preview = clean_text(&text);
        }
    }
    task.updated_at = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
        .unwrap_or_default();
    Ok(task)
}

fn read_index(home: &Path) -> HashMap<String, Value> {
    let mut result = HashMap::new();
    let Ok(contents) = fs::read_to_string(home.join("session_index.jsonl")) else {
        return result;
    };
    for line in contents.lines() {
        if let Ok(item) = serde_json::from_str::<Value>(line) {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                result.insert(id.to_string(), item);
            }
        }
    }
    result
}

fn database_titles(home: &Path) -> HashMap<String, (String, String, bool)> {
    let mut result = HashMap::new();
    let path = home.join("state_5.sqlite");
    let Ok(connection) =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return result;
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT id, COALESCE(title, ''), COALESCE(cwd, ''), COALESCE(archived, 0) FROM threads",
    ) else {
        return result;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
        ))
    }) else {
        return result;
    };
    for row in rows.flatten() {
        result.insert(row.0, (row.1, row.2, row.3));
    }
    result
}

fn catalog_tasks(home: &Path) -> Option<HashMap<String, (String, String, bool)>> {
    let mut result = HashMap::new();
    let path = home.join("sqlite").join("codex-dev.db");
    let Ok(connection) =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return None;
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT thread_id, display_title, cwd, COALESCE(missing_candidate, 0) FROM local_thread_catalog WHERE host_id = 'local'",
    ) else {
        return None;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)? == 0,
        ))
    }) else {
        return None;
    };
    for row in rows.flatten() {
        result.insert(row.0, (row.1, row.2, row.3));
    }
    Some(result)
}

fn value_array_strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn read_desktop_project_state(home: &Path) -> Option<DesktopProjectState> {
    let contents = fs::read_to_string(home.join(".codex-global-state.json")).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let nested_state = value.get("electron-persisted-atom-state");
    let state = if value.get("local-projects").is_some()
        || value.get("thread-project-assignments").is_some()
        || value.get("project-order").is_some()
    {
        &value
    } else {
        nested_state?
    };
    let mut result = DesktopProjectState::default();
    let pinned_ids = value_array_strings(state.get("pinned-project-ids"))
        .into_iter()
        .collect::<HashSet<_>>();

    if let Some(projects) = state.get("local-projects").and_then(Value::as_object) {
        for (id, item) in projects {
            let root_paths = value_array_strings(item.get("rootPaths"));
            if root_paths.is_empty() {
                continue;
            }
            result.projects.insert(
                id.to_string(),
                DesktopProject {
                    id: id.to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| path_name(&root_paths[0])),
                    root_paths,
                    pinned: pinned_ids.contains(id),
                },
            );
        }
    }

    if let Some(assignments) = state
        .get("thread-project-assignments")
        .and_then(Value::as_object)
    {
        for (thread_id, item) in assignments {
            if let Some(project_id) = item.get("projectId").and_then(Value::as_str) {
                result.assignments.insert(
                    thread_id.to_string(),
                    ThreadProjectAssignment {
                        project_id: project_id.to_string(),
                        cwd: item
                            .get("cwd")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    },
                );
            }
        }
    }

    for source in [Some(&value), nested_state] {
        if let Some(items) = source.and_then(Value::as_object) {
            for key in items.keys() {
                if let Some(id) = key.strip_prefix("thread-client-id-v1:local%3A") {
                    result.client_threads.insert(id.to_string());
                }
            }
        }
    }

    Some(result)
}

fn path_name(path: &str) -> String {
    Path::new(path.trim_end_matches(['/', '\\']))
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
}

fn repository_name(url: &str) -> String {
    let mut name = url
        .trim()
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .to_string();
    if name == "." || name == ".." {
        name.clear();
    }
    name
}

fn title_project_hint(title: &str) -> String {
    let token = title.split_whitespace().next().unwrap_or_default();
    if token.len() < 3
        || token
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
    {
        return String::new();
    }
    if token.contains('-') || token.contains('_') || token.contains('.') {
        token.to_string()
    } else {
        String::new()
    }
}

fn is_generic_workspace_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "work" | "workspace" | "projects" | "project" | "documents" | "desktop"
    )
}

fn is_codex_worktree(path: &str) -> bool {
    path.split(['/', '\\'])
        .collect::<Vec<_>>()
        .windows(2)
        .any(|parts| parts == [".codex", "worktrees"])
}

fn first_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.exists()).cloned()
}

fn is_path_in_root(path: &str, root: &str) -> bool {
    let path = Path::new(path);
    let root = Path::new(root);
    path == root || path.starts_with(root)
}

fn project_paths_exist(paths: &[String]) -> bool {
    paths.iter().any(|path| Path::new(path).exists())
}

fn apply_desktop_project(task: &mut Task, project: &DesktopProject) {
    task.project_key = project.id.clone();
    task.project_name = project.name.clone();
    task.project_path = project
        .root_paths
        .first()
        .cloned()
        .unwrap_or_else(|| task.cwd.clone());
    task.project_exists = project_paths_exist(&project.root_paths);
    task.project_pinned = project.pinned;
}

fn matching_desktop_project<'a>(
    state: &'a DesktopProjectState,
    cwd: &str,
    inferred_path: &str,
) -> Option<&'a DesktopProject> {
    state
        .projects
        .values()
        .filter(|project| {
            project
                .root_paths
                .iter()
                .any(|root| is_path_in_root(cwd, root) || is_path_in_root(inferred_path, root))
        })
        .max_by_key(|project| {
            project
                .root_paths
                .iter()
                .map(|root| root.len())
                .max()
                .unwrap_or(0)
        })
}

fn stable_local_project_id(cwd: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in cwd.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("local-{hash:016x}")
}

fn object_mut(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was converted to object")
}

fn array_mut(value: &mut Value) -> &mut Vec<Value> {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    value.as_array_mut().expect("value was converted to array")
}

fn push_unique_string(items: &mut Vec<Value>, value: &str) {
    if !items.iter().any(|item| item.as_str() == Some(value)) {
        items.push(Value::String(value.to_string()));
    }
}

fn client_thread_state_key(thread_id: &str) -> String {
    format!("thread-client-id-v1:local%3A{thread_id}")
}

fn client_thread_state_value(thread_id: &str) -> String {
    format!("client-new-thread:{thread_id}")
}

fn register_desktop_project_state(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let path = home.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(());
    }
    let existing_state = read_desktop_project_state(home).unwrap_or_default();
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(&path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let now_ms = Utc::now().timestamp_millis();
    let root = object_mut(&mut value);
    let use_root = root.contains_key("local-projects")
        || root.contains_key("thread-project-assignments")
        || root.contains_key("project-order");

    for task in tasks {
        let cwd = if task.cwd.trim().is_empty() {
            dirs::home_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            task.cwd.clone()
        };
        let (project_id, project_name, project_roots) = {
            let matched_project = matching_desktop_project(&existing_state, &cwd, &cwd);
            let project_id = matched_project
                .map(|project| project.id.clone())
                .or_else(|| {
                    existing_state
                        .assignments
                        .get(&task.id)
                        .and_then(|assignment| {
                            existing_state
                                .projects
                                .contains_key(&assignment.project_id)
                                .then(|| assignment.project_id.clone())
                        })
                })
                .unwrap_or_else(|| stable_local_project_id(&cwd));
            let project_name = matched_project
                .map(|project| project.name.clone())
                .unwrap_or_else(|| path_name(&cwd));
            let project_roots = matched_project
                .map(|project| project.root_paths.clone())
                .filter(|roots| !roots.is_empty())
                .unwrap_or_else(|| vec![cwd.clone()]);
            (project_id, project_name, project_roots)
        };

        let root = object_mut(&mut value);
        let persisted = if use_root {
            root
        } else {
            object_mut(
                root.entry("electron-persisted-atom-state")
                    .or_insert_with(|| Value::Object(Map::new())),
            )
        };

        let projects = object_mut(
            persisted
                .entry("local-projects")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        projects.insert(
            project_id.clone(),
            serde_json::json!({
                "id": project_id,
                "name": project_name,
                "rootPaths": project_roots,
                "createdAt": now_ms,
                "updatedAt": now_ms
            }),
        );

        let assignments = object_mut(
            persisted
                .entry("thread-project-assignments")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        assignments.insert(
            task.id.clone(),
            serde_json::json!({
                "projectKind": "local",
                "projectId": project_id,
                "cwd": cwd,
                "pendingCoreUpdate": false
            }),
        );

        let project_order = array_mut(
            persisted
                .entry("project-order")
                .or_insert_with(|| Value::Array(Vec::new())),
        );
        push_unique_string(project_order, &project_id);

        let active_roots = array_mut(
            persisted
                .entry("active-workspace-roots")
                .or_insert_with(|| Value::Array(Vec::new())),
        );
        push_unique_string(active_roots, &cwd);

        let root = object_mut(&mut value);
        let atom_state = object_mut(
            root.entry("electron-persisted-atom-state")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        atom_state
            .entry(client_thread_state_key(&task.id))
            .or_insert_with(|| Value::String(client_thread_state_value(&task.id)));

        let workspace_hints = object_mut(
            root.entry("thread-workspace-root-hints")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        workspace_hints.insert(task.id.clone(), Value::String(cwd.clone()));

        let writable_roots = object_mut(
            root.entry("thread-writable-roots")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        let roots = array_mut(
            writable_roots
                .entry(task.id.clone())
                .or_insert_with(|| Value::Array(Vec::new())),
        );
        push_unique_string(roots, &cwd);
    }

    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn infer_project(task: &Task) -> (String, String, String, bool) {
    let cwd = task.cwd.trim();
    if cwd.is_empty() {
        return (
            "__unknown__".to_string(),
            "未记录项目".to_string(),
            String::new(),
            true,
        );
    }

    let repo_name = repository_name(&task.git_origin_url);
    let cwd_name = path_name(cwd);
    let home = dirs::home_dir().unwrap_or_default();
    let project_name = if !repo_name.is_empty() {
        repo_name
    } else if is_generic_workspace_name(&cwd_name) {
        title_project_hint(&task.title)
    } else {
        cwd_name
    };

    if project_name.is_empty() {
        return (
            cwd.to_string(),
            path_name(cwd),
            cwd.to_string(),
            Path::new(cwd).exists(),
        );
    }

    let mut candidates = Vec::new();
    if path_name(cwd) == project_name && !is_codex_worktree(cwd) {
        candidates.push(PathBuf::from(cwd));
    }
    candidates.extend([
        PathBuf::from(cwd).join(&project_name),
        home.join("work").join(&project_name),
        home.join("Projects").join(&project_name),
        home.join("Documents").join(&project_name),
    ]);

    let inferred_path = first_existing_path(&candidates)
        .or_else(|| candidates.first().cloned())
        .unwrap_or_else(|| PathBuf::from(cwd));
    let exists = first_existing_path(&candidates).is_some();
    let key = if task.git_origin_url.trim().is_empty() {
        inferred_path.to_string_lossy().to_string()
    } else {
        task.git_origin_url.clone()
    };

    (
        key,
        project_name,
        inferred_path.to_string_lossy().to_string(),
        exists,
    )
}

fn list_local_tasks() -> Result<Vec<Task>, String> {
    let home = codex_home();
    let index = read_index(&home);
    let database = database_titles(&home);
    let catalog = catalog_tasks(&home);
    let desktop_projects = read_desktop_project_state(&home);
    let mut files = HashSet::new();
    let sessions = home.join("sessions");
    if sessions.exists() {
        for entry in WalkDir::new(sessions).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl")
            {
                files.insert(entry.path().to_path_buf());
            }
        }
    }
    let mut tasks = Vec::new();
    for path in files {
        let Ok(mut task) = session_details(&path) else {
            continue;
        };
        if task.id.is_empty() {
            continue;
        }
        let indexed = index.get(&task.id);
        let database_task = database.get(&task.id);
        let catalog_task = catalog.as_ref().and_then(|items| items.get(&task.id));
        task.title = indexed
            .and_then(|item| item.get("thread_name"))
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .or_else(|| {
                catalog_task
                    .filter(|item| item.2)
                    .map(|item| item.0.clone())
            })
            .or_else(|| database_task.map(|item| item.0.clone()))
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| truncate(&task.first_user_message, 96));
        if task.title.is_empty() {
            task.title = format!("未命名任务 {}", &task.id[..task.id.len().min(8)]);
        }
        if task.updated_at.is_empty() {
            task.updated_at = indexed
                .and_then(|item| item.get("updated_at"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        if task.cwd.is_empty() {
            task.cwd = catalog_task
                .filter(|item| item.2 && !item.1.is_empty())
                .map(|item| item.1.clone())
                .or_else(|| database_task.map(|item| item.1.clone()))
                .unwrap_or_default();
        } else if let Some(catalog_cwd) = catalog_task
            .filter(|item| item.2 && !item.1.is_empty())
            .map(|item| item.1.clone())
        {
            task.cwd = catalog_cwd;
        }
        task.archived = database_task.map(|item| item.2).unwrap_or(false);
        let catalog_visible = catalog
            .as_ref()
            .map(|items| items.get(&task.id).map(|item| item.2).unwrap_or(false))
            .unwrap_or(true);
        task.codex_visible = catalog_visible;
        (
            task.project_key,
            task.project_name,
            task.project_path,
            task.project_exists,
        ) = infer_project(&task);
        if let Some(project_state) = desktop_projects.as_ref() {
            if let Some(assignment) = project_state.assignments.get(&task.id) {
                if let Some(project) = project_state.projects.get(&assignment.project_id) {
                    apply_desktop_project(&mut task, project);
                    task.codex_visible = !task.archived;
                } else {
                    let path = if assignment.cwd.trim().is_empty() {
                        task.cwd.clone()
                    } else {
                        assignment.cwd.clone()
                    };
                    task.project_key = assignment.project_id.clone();
                    task.project_name = path_name(&path);
                    task.project_path = path.clone();
                    task.project_exists = Path::new(&path).exists();
                    task.project_pinned = false;
                    task.codex_visible = false;
                }
            } else if let Some(project) =
                matching_desktop_project(project_state, &task.cwd, &task.project_path)
            {
                apply_desktop_project(&mut task, project);
                task.codex_visible =
                    !task.archived && project_state.client_threads.contains(&task.id);
            } else {
                task.codex_visible = false;
            }
        }
        task.browser_file = home
            .join("browser")
            .join("sessions")
            .join(format!("{}.toml", task.id));
        tasks.push(task);
    }
    tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(tasks)
}

fn archive_path(id: &str, file: &str) -> String {
    format!("tasks/{id}/{file}")
}

fn archive_manifest(path: &Path) -> Result<Manifest, String> {
    let file = File::open(path).map_err(|_| "找不到所选压缩包".to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|_| "这不是有效的 ZIP 压缩包".to_string())?;
    let mut size = 0;
    for index in 0..zip.len() {
        size += zip
            .by_index(index)
            .map_err(|_| "压缩包内容无效".to_string())?
            .size();
    }
    if size > MAX_ARCHIVE_BYTES {
        return Err("压缩包解压后超过 1 GB，已停止导入".to_string());
    }
    let mut contents = String::new();
    zip.by_name("manifest.json")
        .map_err(|_| "压缩包缺少 manifest.json".to_string())?
        .read_to_string(&mut contents)
        .map_err(|error| error.to_string())?;
    let manifest: Manifest =
        serde_json::from_str(&contents).map_err(|_| "压缩包 manifest 无效".to_string())?;
    if manifest.schema != ARCHIVE_SCHEMA
        || manifest.tasks.iter().any(|task| {
            task.task.id.len() < 8
                || !task
                    .session_file
                    .starts_with(&format!("tasks/{}/", task.task.id))
        })
    {
        return Err("这不是有效的 Codex 会话迁移压缩包".to_string());
    }
    Ok(manifest)
}

fn parse_time(value: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
}

fn resolve_local_cwd(cwd: &str) -> String {
    if cwd.is_empty() || Path::new(cwd).exists() {
        return cwd.to_string();
    }
    let name = Path::new(cwd.trim_end_matches(['/', '\\']))
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    for candidate in [
        home.join("work").join(name),
        home.join("Projects").join(name),
        home.join("Documents").join(name),
    ] {
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }
    cwd.to_string()
}

fn replace_value(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => *text = text.replace(from, to),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_value(item, from, to)),
        Value::Object(items) => items
            .values_mut()
            .for_each(|item| replace_value(item, from, to)),
        _ => {}
    }
}

fn rewrite_session_cwd(contents: &str, from: &str, to: &str) -> String {
    if from.is_empty() || from == to {
        return contents.to_string();
    }
    contents
        .lines()
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(mut value) => {
                replace_value(&mut value, from, to);
                serde_json::to_string(&value).unwrap_or_else(|_| line.replace(from, to))
            }
            Err(_) => line.replace(from, to),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn rewrite_session_meta_cwd(contents: &str, to: &str) -> String {
    if to.is_empty() {
        return contents.to_string();
    }
    contents
        .lines()
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(mut value) => {
                if value.get("type").and_then(Value::as_str) == Some("session_meta") {
                    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert("cwd".to_string(), Value::String(to.to_string()));
                    }
                }
                serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
            }
            Err(_) => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn append_index(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let path = home.join("session_index.jsonl");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut ids = HashSet::new();
    for line in existing.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    for task in tasks.iter().filter(|task| !ids.contains(&task.id)) {
        writeln!(file, "{}", serde_json::json!({"id": task.id, "thread_name": task.title, "updated_at": task.updated_at})).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn backup_database(path: &Path, stamp: &str) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let target = PathBuf::from(format!("{}.backup-{stamp}", path.display()));
    fs::copy(path, &target).ok()?;
    Some(target.to_string_lossy().to_string())
}

fn register_threads(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let state = home.join("state_5.sqlite");
    if !state.exists() {
        return Ok(());
    }
    let mut connection = Connection::open(state).map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for task in tasks {
        transaction
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, cwd, title, sandbox_policy, approval_mode, git_branch, git_origin_url, first_user_message, memory_mode, preview, recency_at, recency_at_ms, history_mode, has_user_event, archived, archived_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{\"type\":\"disabled\"}', 'never', ?9, ?10, ?11, 'enabled', ?12, ?4, ?13, 'legacy', 1, 0, NULL) ON CONFLICT(id) DO UPDATE SET rollout_path=excluded.rollout_path, updated_at=excluded.updated_at, source=excluded.source, model_provider=excluded.model_provider, cwd=excluded.cwd, title=excluded.title, git_branch=excluded.git_branch, git_origin_url=excluded.git_origin_url, first_user_message=excluded.first_user_message, preview=excluded.preview, recency_at=excluded.recency_at, recency_at_ms=excluded.recency_at_ms, history_mode=excluded.history_mode, has_user_event=1, archived=0, archived_at=NULL",
                params![
                    task.id,
                    task.file_path.to_string_lossy(),
                    parse_time(&task.created_at).timestamp(),
                    parse_time(&task.updated_at).timestamp(),
                    task.source,
                    task.model_provider,
                    task.cwd,
                    task.title,
                    task.git_branch,
                    task.git_origin_url,
                    task.first_user_message,
                    task.preview,
                    parse_time(&task.updated_at).timestamp_millis()
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

fn register_catalog_threads(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let catalog = home.join("sqlite").join("codex-dev.db");
    if !catalog.exists() {
        return Ok(());
    }
    let mut connection = Connection::open(catalog).map_err(|error| error.to_string())?;
    let has_catalog: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_thread_catalog')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_catalog {
        return Ok(());
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO local_thread_catalog_hosts (host_id, host_kind) VALUES ('local', 'local')",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, 0)",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO local_thread_catalog_sync_state (host_id, observation_sequence, initial_build_complete) VALUES ('local', 0, 1)",
            [],
        )
        .map_err(|error| error.to_string())?;

    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(observation_sequence), 0) FROM local_thread_catalog",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for (index, task) in tasks.iter().enumerate() {
        let created_at = parse_time(&task.created_at).timestamp_millis() as f64 / 1000.0;
        let updated_at = parse_time(&task.updated_at).timestamp_millis() as f64 / 1000.0;
        let source = if task.source.trim().is_empty() {
            "vscode"
        } else {
            task.source.trim()
        };
        let model_provider = if task.model_provider.trim().is_empty() {
            "openai"
        } else {
            task.model_provider.trim()
        };
        let title = if task.title.trim().is_empty() {
            truncate(&task.first_user_message, 96)
        } else {
            task.title.clone()
        };
        transaction
            .execute(
                "INSERT INTO local_thread_catalog (host_id, thread_id, display_title, source_created_at, source_updated_at, cwd, source_kind, source_detail, model_provider, git_branch, observation_sequence, missing_candidate) VALUES ('local', ?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, 0) ON CONFLICT(host_id, thread_id) DO UPDATE SET display_title=excluded.display_title, source_created_at=excluded.source_created_at, source_updated_at=excluded.source_updated_at, cwd=excluded.cwd, source_kind=excluded.source_kind, source_detail=excluded.source_detail, model_provider=excluded.model_provider, git_branch=excluded.git_branch, observation_sequence=excluded.observation_sequence, missing_candidate=0",
                params![
                    task.id,
                    title,
                    created_at,
                    updated_at,
                    task.cwd,
                    source,
                    model_provider,
                    task.git_branch,
                    current + index as i64 + 1
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction
        .execute(
            "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + 1 WHERE id = 1",
            [],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE local_thread_catalog_sync_state SET observation_sequence = MAX(observation_sequence, ?1) WHERE host_id = 'local'",
            params![current + tasks.len() as i64],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn list_tasks() -> Result<TaskList, String> {
    let home = codex_home();
    Ok(TaskList {
        tasks: list_local_tasks()?,
        codex_home: home.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn export_tasks(task_ids: Vec<String>, destination: String) -> Result<serde_json::Value, String> {
    let requested: HashSet<_> = task_ids.into_iter().collect();
    let selected: Vec<_> = list_local_tasks()?
        .into_iter()
        .filter(|task| requested.contains(&task.id))
        .collect();
    if selected.is_empty() {
        return Err("请至少选择一个任务".to_string());
    }
    let destination = PathBuf::from(destination);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = File::create(&destination).map_err(|error| error.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut archive_tasks = Vec::new();
    for task in selected.iter() {
        let session_file = archive_path(&task.id, "session.jsonl");
        zip.start_file(&session_file, options)
            .map_err(|error| error.to_string())?;
        zip.write_all(&fs::read(&task.file_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        let browser_file = if task.browser_file.exists() {
            let archive_file = archive_path(&task.id, "browser.toml");
            zip.start_file(&archive_file, options)
                .map_err(|error| error.to_string())?;
            zip.write_all(&fs::read(&task.browser_file).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            Some(archive_file)
        } else {
            None
        };
        archive_tasks.push(ArchiveTask {
            task: task.clone(),
            session_file,
            browser_file,
        });
    }
    let manifest = Manifest {
        schema: ARCHIVE_SCHEMA.to_string(),
        created_at: Utc::now().to_rfc3339(),
        source_platform: env::consts::OS.to_string(),
        tasks: archive_tasks,
    };
    zip.start_file("manifest.json", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
        )
        .as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok(
        serde_json::json!({"canceled": false, "path": destination, "count": selected.len(), "size": fs::metadata(&destination).map_err(|error| error.to_string())?.len()}),
    )
}

#[tauri::command]
fn inspect_archive(archive_path: String) -> Result<ArchiveInspection, String> {
    let manifest = archive_manifest(Path::new(&archive_path))?;
    let existing: HashSet<_> = list_local_tasks()?
        .into_iter()
        .map(|task| task.id)
        .collect();
    Ok(ArchiveInspection {
        canceled: false,
        path: archive_path,
        created_at: manifest.created_at,
        tasks: manifest
            .tasks
            .into_iter()
            .map(|item| InspectedTask {
                conflict: existing.contains(&item.task.id),
                task: item.task,
            })
            .collect(),
    })
}

#[tauri::command]
fn import_archive(
    archive_path: String,
    options: Option<ImportOptions>,
) -> Result<serde_json::Value, String> {
    if is_codex_desktop_running() {
        return Err("检测到 Codex/ChatGPT 桌面端正在运行。请先完全退出 Codex，再执行导入或恢复，避免侧边栏状态被运行中的客户端覆盖。".to_string());
    }
    let source = PathBuf::from(&archive_path);
    let manifest = archive_manifest(&source)?;
    let home = codex_home();
    fs::create_dir_all(home.join("sessions")).map_err(|error| error.to_string())?;
    fs::create_dir_all(home.join("browser").join("sessions")).map_err(|error| error.to_string())?;
    let existing_tasks: HashMap<_, _> = list_local_tasks()?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    let file = File::open(source).map_err(|error| error.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|error| error.to_string())?;
    let adapt_paths = options
        .as_ref()
        .and_then(|value| value.adapt_paths)
        .unwrap_or(true);
    let restore_existing = options
        .as_ref()
        .and_then(|value| value.restore_existing)
        .unwrap_or(false);
    let target_cwd = options
        .as_ref()
        .and_then(|value| value.target_cwd.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let mut imported = Vec::new();
    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    let mut backups = Vec::new();
    for entry in manifest.tasks {
        if let Some(existing_task) = existing_tasks.get(&entry.task.id) {
            if restore_existing {
                let mut task = existing_task.clone();
                if task.title.is_empty() {
                    task.title = entry.task.title;
                }
                if task.first_user_message.is_empty() {
                    task.first_user_message = entry.task.first_user_message;
                }
                if task.preview.is_empty() {
                    task.preview = entry.task.preview;
                }
                if task.source.is_empty() {
                    task.source = entry.task.source;
                }
                if task.model_provider.is_empty() {
                    task.model_provider = entry.task.model_provider;
                }
                if let Some(cwd) = target_cwd.as_deref() {
                    if let Ok(contents) = fs::read_to_string(&task.file_path) {
                        if let Some(backup) = backup_database(&task.file_path, &stamp) {
                            backups.push(backup);
                        }
                        fs::write(&task.file_path, rewrite_session_meta_cwd(&contents, cwd))
                            .map_err(|error| error.to_string())?;
                    }
                    task.cwd = cwd.to_string();
                }
                task.archived = false;
                (
                    task.project_key,
                    task.project_name,
                    task.project_path,
                    task.project_exists,
                ) = infer_project(&task);
                restored.push(task);
            } else {
                skipped.push(serde_json::json!({"id": entry.task.id, "title": entry.task.title, "reason": "already_exists"}));
            }
            continue;
        }
        let mut task = entry.task;
        let date = parse_time(if task.created_at.is_empty() {
            &task.updated_at
        } else {
            &task.created_at
        });
        let target_dir = home
            .join("sessions")
            .join(date.year().to_string())
            .join(format!("{:02}", date.month()))
            .join(format!("{:02}", date.day()));
        fs::create_dir_all(&target_dir).map_err(|error| error.to_string())?;
        let rollout_path = target_dir.join(format!(
            "rollout-{}-{}.jsonl",
            date.format("%Y-%m-%dT%H-%M-%S"),
            task.id
        ));
        let mut content = String::new();
        zip.by_name(&entry.session_file)
            .map_err(|_| "压缩包缺少会话文件".to_string())?
            .read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        let local_cwd = if let Some(cwd) = target_cwd.as_deref() {
            cwd.to_string()
        } else if adapt_paths {
            resolve_local_cwd(&task.cwd)
        } else {
            task.cwd.clone()
        };
        let mut session_content = rewrite_session_cwd(&content, &task.cwd, &local_cwd);
        if target_cwd.is_some() {
            session_content = rewrite_session_meta_cwd(&session_content, &local_cwd);
        }
        fs::write(&rollout_path, session_content).map_err(|error| error.to_string())?;
        if let Some(browser_file) = entry.browser_file {
            let mut contents = Vec::new();
            zip.by_name(&browser_file)
                .map_err(|_| "压缩包缺少浏览器配置".to_string())?
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            fs::write(
                home.join("browser")
                    .join("sessions")
                    .join(format!("{}.toml", task.id)),
                contents,
            )
            .map_err(|error| error.to_string())?;
        }
        task.cwd = if local_cwd.is_empty() {
            dirs::home_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            local_cwd
        };
        task.file_path = rollout_path;
        if task.updated_at.is_empty() {
            task.updated_at = Utc::now().to_rfc3339();
        }
        if task.created_at.is_empty() {
            task.created_at = task.updated_at.clone();
        }
        if task.title.is_empty() {
            task.title = truncate(&task.first_user_message, 96);
        }
        (
            task.project_key,
            task.project_name,
            task.project_path,
            task.project_exists,
        ) = infer_project(&task);
        imported.push(task);
    }
    if imported.is_empty() && restored.is_empty() {
        return Ok(
            serde_json::json!({"imported": [], "restored": [], "skipped": skipped, "backups": [], "codexHome": home}),
        );
    }
    backups.extend(
        [
            backup_database(&home.join("state_5.sqlite"), &stamp),
            backup_database(&home.join("sqlite").join("codex-dev.db"), &stamp),
            backup_database(&home.join(".codex-global-state.json"), &stamp),
        ]
        .into_iter()
        .flatten(),
    );
    let registered = imported
        .iter()
        .chain(restored.iter())
        .cloned()
        .collect::<Vec<_>>();
    append_index(&home, &registered)?;
    register_threads(&home, &registered)?;
    register_catalog_threads(&home, &registered)?;
    register_desktop_project_state(&home, &registered)?;
    Ok(
        serde_json::json!({"imported": imported.iter().map(|task| serde_json::json!({"id": task.id, "title": task.title, "cwd": task.cwd, "rolloutPath": task.file_path})).collect::<Vec<_>>(), "restored": restored.iter().map(|task| serde_json::json!({"id": task.id, "title": task.title, "cwd": task.cwd, "rolloutPath": task.file_path})).collect::<Vec<_>>(), "skipped": skipped, "backups": backups, "codexHome": home}),
    )
}

#[tauri::command]
fn get_environment() -> serde_json::Value {
    let codex_processes = codex_desktop_processes();
    serde_json::json!({"codexHome": codex_home(), "platform": env::consts::OS, "version": env!("CARGO_PKG_VERSION"), "codexRunning": !codex_processes.is_empty(), "codexProcesses": codex_processes})
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_tasks,
            export_tasks,
            inspect_archive,
            import_archive,
            get_environment
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Session Transfer");
}

#[cfg(test)]
mod tests {
    use super::{infer_project, repository_name, rewrite_session_cwd, Task};
    use std::{env, path::PathBuf};

    #[test]
    fn rewrite_session_cwd_updates_nested_json() {
        let source = "{\"payload\":{\"cwd\":\"/old\",\"values\":[\"/old/file\"]}}\n";
        assert!(rewrite_session_cwd(source, "/old", "/new").contains("/new/file"));
    }

    fn task_with(title: &str, cwd: &str, git_origin_url: &str) -> Task {
        Task {
            id: "test-task".to_string(),
            title: title.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            cwd: cwd.to_string(),
            project_key: String::new(),
            project_name: String::new(),
            project_path: String::new(),
            source: String::new(),
            model_provider: String::new(),
            git_branch: String::new(),
            git_origin_url: git_origin_url.to_string(),
            first_user_message: String::new(),
            preview: String::new(),
            message_count: 0,
            user_message_count: 0,
            size: 0,
            archived: false,
            project_exists: true,
            codex_visible: true,
            project_pinned: false,
            file_path: PathBuf::new(),
            browser_file: PathBuf::new(),
        }
    }

    #[test]
    fn repository_name_handles_ssh_urls() {
        assert_eq!(
            repository_name("git@github.com:myyimu/ai-novel-diagnosis.git"),
            "ai-novel-diagnosis"
        );
    }

    #[test]
    fn infer_project_marks_missing_child_under_generic_workspace() {
        let cwd = env::temp_dir().join("work");
        let name = format!("codex-session-transfer-missing-{}", std::process::id());
        let task = task_with(&format!("{name} 继续整理"), &cwd.to_string_lossy(), "");
        let (_, project_name, project_path, exists) = infer_project(&task);
        assert_eq!(project_name, name);
        assert!(project_path.ends_with(&project_name));
        assert!(!exists);
    }

    #[test]
    fn infer_project_ignores_codex_worktree_as_real_project() {
        let cwd = env::temp_dir()
            .join(".codex")
            .join("worktrees")
            .join("abcd")
            .join("deleted-project");
        let task = task_with(
            "继续",
            &cwd.to_string_lossy(),
            "git@github.com:myyimu/deleted-project.git",
        );
        let (_, project_name, _, exists) = infer_project(&task);
        assert_eq!(project_name, "deleted-project");
        assert!(!exists);
    }
}
