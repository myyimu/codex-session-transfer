use chrono::{DateTime, Datelike, Local, Utc};
#[cfg(not(target_os = "windows"))]
use chrono::{NaiveDateTime, TimeZone};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant, SystemTime},
};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const ARCHIVE_SCHEMA: &str = "codex-session-transfer/v1";
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const TASK_SCAN_TIMEOUT: Duration = Duration::from_secs(90);
const PAUSED_TASK_SCAN_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_PAUSED_TASK_SCANS: usize = 3;
const MAX_SESSION_DETAILS_CACHE_ENTRIES: usize = 2_048;
const LOCAL_WRITE_LOCK_FILE: &str = ".codex-session-transfer-write.lock";
const LOCAL_WRITE_LOCK_TTL: Duration = Duration::from_secs(2 * 60);
const LOCAL_SNAPSHOT_MARKER: &str = ".codex-session-transfer.backup-";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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
    #[serde(default)]
    model: String,
    #[serde(default)]
    reasoning_effort: String,
    #[serde(default)]
    sandbox_policy: String,
    #[serde(default)]
    approval_mode: String,
    #[serde(default)]
    cli_version: String,
    #[serde(default)]
    thread_source: String,
    #[serde(default)]
    forked_from_id: String,
    #[serde(default)]
    agent_path: String,
    #[serde(default)]
    agent_nickname: String,
    #[serde(default)]
    agent_role: String,
    #[serde(default)]
    memory_mode: String,
    #[serde(default)]
    history_mode: String,
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
    bad_title_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalSnapshot {
    path: String,
    name: String,
    size: u64,
    modified_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDeletionResult {
    deleted_count: usize,
    reclaimed_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskHealthIssue {
    code: String,
    level: String,
    title: String,
    detail: String,
    recommended_action: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskHealthItem {
    id: String,
    title: String,
    cwd: String,
    issues: Vec<TaskHealthIssue>,
    safe_actions: Vec<String>,
    requires_manual_review: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskHealthSummary {
    healthy_count: usize,
    reregister_count: usize,
    title_repair_count: usize,
    manual_review_count: usize,
    unbound_project_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskHealthReport {
    summary: TaskHealthSummary,
    tasks: Vec<TaskHealthItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskLibrary {
    tasks: Vec<Task>,
    codex_home: String,
    health: TaskHealthReport,
}

// A scan may be stopped between files.  We intentionally do not abort a write
// operation halfway through: repair/import cancellation is only safe before a
// snapshot or database write begins.
static CANCELLED_BACKGROUND_JOBS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static PAUSED_TASK_SCANS: OnceLock<Mutex<HashMap<String, PausedTaskScan>>> = OnceLock::new();
static LOCAL_WRITE_OPERATION: OnceLock<Mutex<()>> = OnceLock::new();

struct PausedTaskScan {
    files: Vec<PathBuf>,
    next_index: usize,
    seen_task_ids: HashSet<String>,
    tasks: Vec<Task>,
    discovered: usize,
    paused_at: Instant,
}

fn cancelled_background_jobs() -> &'static Mutex<HashSet<String>> {
    CANCELLED_BACKGROUND_JOBS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn job_is_cancelled(job_id: &str) -> bool {
    cancelled_background_jobs()
        .lock()
        .map(|jobs| jobs.contains(job_id))
        .unwrap_or(false)
}

fn clear_background_job(job_id: &str) {
    if let Ok(mut jobs) = cancelled_background_jobs().lock() {
        jobs.remove(job_id);
    }
}

fn paused_task_scans() -> &'static Mutex<HashMap<String, PausedTaskScan>> {
    PAUSED_TASK_SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct LocalWriteOperation {
    _in_process: MutexGuard<'static, ()>,
    lock_path: PathBuf,
}

impl Drop for LocalWriteOperation {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

fn write_lock_is_stale(lock_path: &Path, now: SystemTime) -> bool {
    let expired = fs::metadata(lock_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > LOCAL_WRITE_LOCK_TTL);
    expired && !lock_owner_is_running(lock_path)
}

fn lock_owner_is_running(lock_path: &Path) -> bool {
    let Some((pid, started_at)) = read_lock_owner(lock_path)
    else {
        return false;
    };
    let is_running = process_is_running(pid);
    if !is_running {
        return false;
    }
    started_at
        .zip(process_started_at(pid))
        .map(|(expected, actual)| expected == actual)
        .unwrap_or(true)
}

fn read_lock_owner(lock_path: &Path) -> Option<(u32, Option<i64>)> {
    let contents = fs::read_to_string(lock_path).ok()?;
    let pid = contents.lines().find_map(|line| {
        line.strip_prefix("pid=")
            .and_then(|value| value.trim().parse::<u32>().ok())
    })?;
    let started_at = contents.lines().find_map(|line| {
        line.strip_prefix("started_at_unix=")
            .and_then(|value| value.trim().parse::<i64>().ok())
    });
    Some((pid, started_at))
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("tasklist");
        command
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW);
        command
            .output()
            .ok()
            .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn process_started_at(pid: u32) -> Option<i64> {
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$process = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($process) {{ ([DateTimeOffset]$process.StartTime).ToUnixTimeSeconds() }}"
        );
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW);
        command
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|output| output.trim().parse::<i64>().ok())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "lstart="])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|output| NaiveDateTime::parse_from_str(output.trim(), "%a %b %e %H:%M:%S %Y").ok())
            .and_then(|started_at| Local.from_local_datetime(&started_at).single())
            .map(|started_at| started_at.timestamp())
    }
}

fn write_lock_contents(lock_path: &Path, mut file: File, contents: &[u8]) -> Result<(), String> {
    if let Err(error) = file.write_all(contents) {
        fs::remove_file(lock_path).ok();
        return Err(error.to_string());
    }
    Ok(())
}

fn create_local_write_lock(lock_path: &Path) -> Result<(), String> {
    let contents = format!(
        "pid={}\nstarted_at_unix={}\n",
        std::process::id(),
        process_started_at(std::process::id()).unwrap_or_else(|| Utc::now().timestamp()),
    );
    match OpenOptions::new().write(true).create_new(true).open(lock_path) {
        Ok(file) => write_lock_contents(lock_path, file, contents.as_bytes()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists && write_lock_is_stale(lock_path, SystemTime::now()) => {
            fs::remove_file(lock_path).map_err(|error| error.to_string())?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(lock_path)
                .map_err(|error| error.to_string())?;
            write_lock_contents(lock_path, file, contents.as_bytes())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(
            "另一实例正在写入 Codex 本地数据。请等待其完成；若确认程序已异常退出，约两分钟后可自动恢复重试。".to_string(),
        ),
        Err(error) => Err(error.to_string()),
    }
}

fn acquire_local_write_operation(home: &Path) -> Result<LocalWriteOperation, String> {
    let in_process = LOCAL_WRITE_OPERATION
        .get_or_init(|| Mutex::new(()))
        .try_lock()
        .map_err(|_| "已有本地导入或修复正在执行，请等待其完成。".to_string())?;
    fs::create_dir_all(home).map_err(|error| error.to_string())?;
    let lock_path = home.join(LOCAL_WRITE_LOCK_FILE);
    create_local_write_lock(&lock_path)?;
    Ok(LocalWriteOperation {
        _in_process: in_process,
        lock_path,
    })
}

fn prune_paused_task_scans(scans: &mut HashMap<String, PausedTaskScan>) {
    scans.retain(|_, scan| scan.paused_at.elapsed() < PAUSED_TASK_SCAN_TTL);
    while scans.len() > MAX_PAUSED_TASK_SCANS {
        let Some(oldest) = scans
            .iter()
            .min_by_key(|(_, scan)| scan.paused_at)
            .map(|(token, _)| token.clone())
        else {
            break;
        };
        scans.remove(&oldest);
    }
}

fn take_paused_task_scan(token: &str) -> Option<PausedTaskScan> {
    let mut scans = paused_task_scans().lock().ok()?;
    prune_paused_task_scans(&mut scans);
    scans.remove(token)
}

fn store_paused_task_scan(token: String, scan: PausedTaskScan) {
    let Ok(mut scans) = paused_task_scans().lock() else {
        return;
    };
    scans.insert(token, scan);
    prune_paused_task_scans(&mut scans);
}

#[derive(Clone)]
struct SessionDetailsCacheEntry {
    size: u64,
    modified_at: Option<SystemTime>,
    task: Task,
}

#[derive(Default)]
struct SessionDetailsCache {
    entries: HashMap<PathBuf, SessionDetailsCacheEntry>,
    insertion_order: VecDeque<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairPlanItem {
    id: String,
    title: String,
    cwd: String,
    actions: Vec<String>,
    can_apply: bool,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairPlan {
    items: Vec<RepairPlanItem>,
    actionable_count: usize,
    manual_review_count: usize,
    snapshot_note: String,
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
    merge_preview: Option<SessionMergePreview>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionMergePreview {
    can_merge: bool,
    archive_record_count: usize,
    local_record_count: usize,
    append_record_count: usize,
    archive_last_activity: String,
    local_last_activity: String,
    reason: String,
}

#[derive(Clone, Debug)]
struct SessionMergeResult {
    preview: SessionMergePreview,
    contents: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportOptions {
    adapt_paths: Option<bool>,
    restore_existing: Option<bool>,
    merge_task_ids: Option<Vec<String>>,
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

#[derive(Clone, Debug)]
struct DatabaseTask {
    title: String,
    cwd: String,
    first_user_message: String,
    archived: bool,
    source: String,
    model_provider: String,
    model: String,
    reasoning_effort: String,
    sandbox_policy: String,
    approval_mode: String,
    cli_version: String,
    thread_source: String,
    agent_path: String,
    agent_nickname: String,
    agent_role: String,
    memory_mode: String,
    history_mode: String,
}

#[derive(Clone, Debug)]
struct CodexModelSettings {
    provider: String,
    model: String,
    reasoning_effort: String,
}

#[derive(Clone, Debug, Default)]
struct DesktopProjectState {
    projects: HashMap<String, DesktopProject>,
    project_order: Vec<String>,
    assignments: HashMap<String, ThreadProjectAssignment>,
    projectless_threads: HashSet<String>,
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

fn latest_state_database(home: &Path) -> PathBuf {
    let mut candidates = fs::read_dir(home)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            let modified = entry.metadata().ok()?.modified().ok()?;
            let modified = modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some((version, modified, name.to_string(), path))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates
        .pop()
        .map(|(_, _, _, path)| path)
        .unwrap_or_else(|| home.join("state_5.sqlite"))
}

fn history_first_messages(home: &Path) -> HashMap<String, String> {
    let mut messages = HashMap::new();
    let Ok(contents) = fs::read_to_string(home.join("history.jsonl")) else {
        return messages;
    };
    for line in contents.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = value.get("session_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = value
            .get("text")
            .and_then(Value::as_str)
            .and_then(meaningful_user_text)
        else {
            continue;
        };
        messages.entry(id.to_string()).or_insert(text);
    }
    messages
}

fn simple_toml_string(contents: &str, key: &str) -> Option<String> {
    let mut at_root = true;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            at_root = false;
            continue;
        }
        if !at_root || !line.starts_with(key) {
            continue;
        }
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if left.trim() != key {
            continue;
        }
        let value = toml_value_without_comment(right)
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn toml_value_without_comment(value: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if active_quote == '"' && character == '\\' && !escaped {
                escaped = true;
                continue;
            }
            if character == active_quote && !escaped {
                quote = None;
            }
            escaped = false;
        } else if matches!(character, '"' | '\'') {
            quote = Some(character);
        } else if character == '#' {
            return &value[..index];
        }
    }
    value
}

fn latest_session_model_settings(home: &Path) -> Option<CodexModelSettings> {
    let mut files = WalkDir::new(home.join("sessions"))
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("jsonl")
        })
        .map(|entry| entry.path().to_path_buf())
        .collect::<Vec<_>>();
    // Rollout filenames start with the creation time, so descending path order
    // yields the newest session without being affected by restore file writes.
    files.sort_unstable_by(|left, right| right.cmp(left));

    for path in files {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut settings = CodexModelSettings {
            provider: String::new(),
            model: String::new(),
            reasoning_effort: String::new(),
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let record_type = record.get("type").and_then(Value::as_str);
            let payload = record.get("payload").unwrap_or(&Value::Null);
            if record_type == Some("session_meta") {
                set_if_empty(
                    &mut settings.provider,
                    payload.get("model_provider").and_then(Value::as_str),
                );
                set_if_empty(
                    &mut settings.model,
                    payload.get("model").and_then(Value::as_str),
                );
                set_if_empty(
                    &mut settings.reasoning_effort,
                    payload.get("reasoning_effort").and_then(Value::as_str),
                );
            }
            if record_type == Some("turn_context") {
                set_if_empty(
                    &mut settings.model,
                    payload.get("model").and_then(Value::as_str),
                );
                set_if_empty(
                    &mut settings.reasoning_effort,
                    payload
                        .get("reasoning_effort")
                        .and_then(Value::as_str)
                        .or_else(|| payload.get("effort").and_then(Value::as_str)),
                );
            }
            if payload.get("type").and_then(Value::as_str) == Some("thread_settings_applied") {
                let thread_settings = payload.get("thread_settings").unwrap_or(&Value::Null);
                let provider = thread_settings
                    .get("model_provider_id")
                    .or_else(|| thread_settings.get("model_provider"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let model = thread_settings
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let effort = thread_settings
                    .get("reasoning_effort")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                // A provider switch is atomic: accepting only its provider would
                // combine it with a model recorded under a different endpoint.
                if let (Some(provider), Some(model)) = (provider, model) {
                    settings.provider = provider.to_string();
                    settings.model = model.to_string();
                    replace_if_present(&mut settings.reasoning_effort, effort.unwrap_or_default());
                } else if provider.is_none() {
                    replace_if_present(&mut settings.model, model.unwrap_or_default());
                    replace_if_present(&mut settings.reasoning_effort, effort.unwrap_or_default());
                }
            }
        }
        if !settings.provider.is_empty() && !settings.model.is_empty() {
            return Some(settings);
        }
    }
    None
}

fn cached_environment_model_settings(home: &Path) -> CodexModelSettings {
    static CACHE: OnceLock<Mutex<Option<(PathBuf, Instant, CodexModelSettings)>>> = OnceLock::new();
    const TTL: Duration = Duration::from_secs(10);
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((cached_home, updated_at, settings)) = guard.as_ref() {
            if cached_home == home && updated_at.elapsed() < TTL {
                return settings.clone();
            }
        }
    }
    let settings = codex_model_settings(home);
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((home.to_path_buf(), Instant::now(), settings.clone()));
    }
    settings
}

fn codex_model_settings(home: &Path) -> CodexModelSettings {
    let config = fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let config_provider = simple_toml_string(&config, "model_provider");
    let config_model = simple_toml_string(&config, "model");
    let config_effort = simple_toml_string(&config, "model_reasoning_effort");
    let session_settings = config_provider
        .is_none()
        .then(|| latest_session_model_settings(home))
        .flatten();
    // A provider inferred from a session is only meaningful together with that
    // session's model/effort.  Do not combine it with stale config defaults.
    let session_model = || {
        session_settings
            .as_ref()
            .map(|settings| settings.model.clone())
            .filter(|model| !model.trim().is_empty())
    };
    let session_effort = || {
        session_settings
            .as_ref()
            .map(|settings| settings.reasoning_effort.clone())
            .filter(|effort| !effort.trim().is_empty())
    };
    let model = (if config_provider.is_some() {
        config_model.or_else(session_model)
    } else {
        session_model().or(config_model)
    })
    .unwrap_or_else(|| "gpt-5.5".to_string());
    let reasoning_effort = (if config_provider.is_some() {
        config_effort.or_else(session_effort)
    } else {
        session_effort().or(config_effort)
    })
    .unwrap_or_else(|| "high".to_string());
    CodexModelSettings {
        provider: config_provider
            .or_else(|| session_settings.map(|settings| settings.provider))
            .filter(|provider| !provider.trim().is_empty())
            .unwrap_or_else(|| "openai".to_string()),
        model,
        reasoning_effort,
    }
}

fn codex_desktop_processes() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("tasklist");
        command
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW);
        let output = command.output().ok();
        let text = output
            .as_ref()
            .map(|item| String::from_utf8_lossy(&item.stdout).to_string())
            .unwrap_or_default();
        text
            .lines()
            .filter_map(|line| line.split(',').next())
            .map(|name| name.trim().trim_matches('"').to_string())
            .filter(|name| is_codex_desktop_process(name))
            .collect()
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
            .filter(|line| is_codex_desktop_process(line))
            .map(|line| clean_text(line))
            .collect()
    }
}

fn is_codex_desktop_process(value: &str) -> bool {
    matches!(value, "ChatGPT.exe" | "Codex.exe")
        || value.contains("ChatGPT.app/Contents/MacOS/ChatGPT")
        || value.contains("Codex.app/Contents/MacOS/Codex")
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

fn is_codex_context_text(value: &str) -> bool {
    let value = clean_text(value);
    let lower = value.to_ascii_lowercase();
    lower.starts_with("<environment_context")
        || lower.starts_with("<recommended_plugins")
        || lower.starts_with("[tool")
        || lower.starts_with("assistant to=")
}

fn is_bad_title(value: &str) -> bool {
    let value = clean_text(value);
    if value.is_empty() {
        return true;
    }
    is_codex_context_text(&value)
        || value.chars().count() > 180
        || ["<image name=", "assistant to=", "tool exec call:"]
            .iter()
            .any(|marker| value.to_ascii_lowercase().contains(marker))
}

fn meaningful_user_text(value: &str) -> Option<String> {
    let cleaned = clean_text(value);
    let lower = cleaned.to_ascii_lowercase();
    let end = [
        "<image name=",
        "<environment_context",
        "<recommended_plugins",
        "assistant to=",
        "[tool",
        "tool exec call:",
    ]
    .iter()
    .filter_map(|marker| lower.find(marker))
    .min()
    .unwrap_or(cleaned.len());
    let value = cleaned[..end].trim().to_string();
    if value.is_empty() || is_codex_context_text(&value) {
        None
    } else {
        Some(value)
    }
}

fn normalize_task_title(task: &mut Task) {
    if is_bad_title(&task.title) {
        task.title = meaningful_user_text(&task.first_user_message)
            .map(|message| truncate(&message, 96))
            .unwrap_or_default();
    }
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

fn nested_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in keys {
        current = current.get(*key)?;
    }
    current.as_str().filter(|text| !text.trim().is_empty())
}

fn set_if_empty(target: &mut String, value: Option<&str>) {
    if target.is_empty() {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            *target = value.to_string();
        }
    }
}

fn set_json_if_empty(target: &mut String, value: Option<&Value>) {
    if target.is_empty() {
        if let Some(value) = value {
            if !value.is_null() {
                *target = serde_json::to_string(value).unwrap_or_default();
            }
        }
    }
}

fn apply_session_record_metadata(task: &mut Task, record: &Value) {
    let record_type = record.get("type").and_then(Value::as_str);
    let payload = record.get("payload").unwrap_or(&Value::Null);

    if record_type == Some("session_meta") {
        set_if_empty(&mut task.id, payload.get("id").and_then(Value::as_str));
        set_if_empty(
            &mut task.created_at,
            payload
                .get("timestamp")
                .or_else(|| record.get("timestamp"))
                .and_then(Value::as_str),
        );
        set_if_empty(&mut task.cwd, payload.get("cwd").and_then(Value::as_str));
        set_if_empty(
            &mut task.source,
            payload.get("source").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.model_provider,
            payload.get("model_provider").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.model,
            payload.get("model").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.reasoning_effort,
            payload.get("reasoning_effort").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.sandbox_policy,
            payload.get("sandbox_policy").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.approval_mode,
            payload.get("approval_mode").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.cli_version,
            payload.get("cli_version").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.thread_source,
            payload.get("thread_source").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.forked_from_id,
            payload.get("forked_from_id").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.agent_path,
            payload.get("agent_path").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.agent_nickname,
            payload.get("agent_nickname").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.agent_role,
            payload.get("agent_role").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.memory_mode,
            payload.get("memory_mode").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.history_mode,
            payload.get("history_mode").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.git_branch,
            nested_string(payload, &["git", "branch"]),
        );
        set_if_empty(
            &mut task.git_origin_url,
            nested_string(payload, &["git", "repository_url"]),
        );
    }

    if record_type == Some("turn_context") {
        set_if_empty(&mut task.cwd, payload.get("cwd").and_then(Value::as_str));
        set_json_if_empty(&mut task.sandbox_policy, payload.get("sandbox_policy"));
        set_if_empty(
            &mut task.approval_mode,
            payload.get("approval_policy").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.model,
            payload
                .get("model")
                .and_then(Value::as_str)
                .or_else(|| nested_string(payload, &["collaboration_mode", "settings", "model"])),
        );
        set_if_empty(
            &mut task.reasoning_effort,
            payload
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .or_else(|| payload.get("effort").and_then(Value::as_str))
                .or_else(|| {
                    nested_string(
                        payload,
                        &["collaboration_mode", "settings", "reasoning_effort"],
                    )
                }),
        );
    }

    if payload.get("type").and_then(Value::as_str) == Some("thread_settings_applied") {
        let settings = payload.get("thread_settings").unwrap_or(&Value::Null);
        set_if_empty(&mut task.cwd, settings.get("cwd").and_then(Value::as_str));
        set_json_if_empty(&mut task.sandbox_policy, settings.get("sandbox_policy"));
        set_if_empty(
            &mut task.approval_mode,
            settings.get("approval_policy").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.model_provider,
            settings
                .get("model_provider_id")
                .or_else(|| settings.get("model_provider"))
                .and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.model,
            settings.get("model").and_then(Value::as_str),
        );
        set_if_empty(
            &mut task.reasoning_effort,
            settings.get("reasoning_effort").and_then(Value::as_str),
        );
    }
}

fn hydrate_task_from_session_content(task: &mut Task, contents: &str) {
    for line in contents.lines() {
        if let Ok(record) = serde_json::from_str::<Value>(line) {
            apply_session_record_metadata(task, &record);
        }
    }
}

fn session_details(path: &Path) -> Result<Task, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    let size = metadata.len();
    let modified_at = metadata.modified().ok();
    static CACHE: OnceLock<Mutex<SessionDetailsCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(SessionDetailsCache::default()));
    if let Ok(cache) = cache.lock() {
        if let Some(entry) = cache.entries.get(path) {
            if entry.size == size && entry.modified_at == modified_at {
                return Ok(entry.task.clone());
            }
        }
    }
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
        model: String::new(),
        reasoning_effort: String::new(),
        sandbox_policy: String::new(),
        approval_mode: String::new(),
        cli_version: String::new(),
        thread_source: String::new(),
        forked_from_id: String::new(),
        agent_path: String::new(),
        agent_nickname: String::new(),
        agent_role: String::new(),
        memory_mode: String::new(),
        history_mode: String::new(),
        git_branch: String::new(),
        git_origin_url: String::new(),
        first_user_message: String::new(),
        preview: String::new(),
        message_count: 0,
        user_message_count: 0,
        size,
        archived: false,
        project_exists: true,
        codex_visible: true,
        project_pinned: false,
        file_path: path.to_path_buf(),
        browser_file: PathBuf::new(),
    };
    let mut last_activity_at = String::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        apply_session_record_metadata(&mut task, &record);
        // Track the most recent timestamp seen across rollout records so the
        // displayed session time reflects the last real conversation activity
        // instead of the rollout file's modification time (which becomes the
        // restore moment for imported sessions).
        if let Some(timestamp) = record
            .get("timestamp")
            .and_then(Value::as_str)
            .or_else(|| {
                record
                    .get("payload")
                    .and_then(|payload| payload.get("timestamp"))
                    .and_then(Value::as_str)
            })
            .filter(|timestamp| !timestamp.trim().is_empty())
        {
            last_activity_at = timestamp.to_string();
        }
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
            if task.model.is_empty() {
                task.model = value_string(payload.get("model"));
            }
            if task.reasoning_effort.is_empty() {
                task.reasoning_effort = value_string(payload.get("reasoning_effort"));
            }
            if task.cli_version.is_empty() {
                task.cli_version = value_string(payload.get("cli_version"));
            }
            if task.thread_source.is_empty() {
                task.thread_source = value_string(payload.get("thread_source"));
            }
            if task.agent_path.is_empty() {
                task.agent_path = value_string(payload.get("agent_path"));
            }
            if task.agent_nickname.is_empty() {
                task.agent_nickname = value_string(payload.get("agent_nickname"));
            }
            if task.agent_role.is_empty() {
                task.agent_role = value_string(payload.get("agent_role"));
            }
            if task.memory_mode.is_empty() {
                task.memory_mode = value_string(payload.get("memory_mode"));
            }
            if task.history_mode.is_empty() {
                task.history_mode = value_string(payload.get("history_mode"));
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
            if let Some(text) = meaningful_user_text(&text) {
                task.user_message_count += 1;
                if task.first_user_message.is_empty() {
                    task.first_user_message = text.clone();
                }
                task.preview = text;
            }
        }
    }
    // Prefer the last in-content activity time; only fall back to the file
    // modification time when the rollout carried no record timestamps.
    task.updated_at = if last_activity_at.is_empty() {
        fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
            .unwrap_or_default()
    } else {
        DateTime::parse_from_rfc3339(&last_activity_at)
            .map(|time| time.with_timezone(&Utc).to_rfc3339())
            .unwrap_or(last_activity_at)
    };
    if let Ok(mut cache) = cache.lock() {
        if !cache.entries.contains_key(path) {
            while cache.entries.len() >= MAX_SESSION_DETAILS_CACHE_ENTRIES {
                if let Some(oldest) = cache.insertion_order.pop_front() {
                    cache.entries.remove(&oldest);
                } else {
                    break;
                }
            }
            cache.insertion_order.push_back(path.to_path_buf());
        }
        cache.entries.insert(
            path.to_path_buf(),
            SessionDetailsCacheEntry {
                size,
                modified_at,
                task: task.clone(),
            },
        );
    }
    Ok(task)
}

fn record_timestamp(record: &Value) -> Option<DateTime<Utc>> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| {
            record
                .get("payload")
                .and_then(|payload| payload.get("timestamp"))
                .and_then(Value::as_str)
        })
        .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn safe_merge_session_jsonl(archive: &str, local: &str) -> SessionMergeResult {
    let archive_lines = archive
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let local_lines = local
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let base_preview = |can_merge: bool,
                        append_record_count: usize,
                        archive_last: Option<DateTime<Utc>>,
                        local_last: Option<DateTime<Utc>>,
                        reason: &str| {
        SessionMergePreview {
            can_merge,
            archive_record_count: archive_lines.len(),
            local_record_count: local_lines.len(),
            append_record_count,
            archive_last_activity: archive_last
                .map(|time| time.to_rfc3339())
                .unwrap_or_default(),
            local_last_activity: local_last.map(|time| time.to_rfc3339()).unwrap_or_default(),
            reason: reason.to_string(),
        }
    };
    if archive_lines.is_empty() || local_lines.is_empty() {
        return SessionMergeResult {
            preview: base_preview(false, 0, None, None, "归档或本机会话为空，无法安全比较。"),
            contents: String::new(),
        };
    }

    let mut archive_last: Option<DateTime<Utc>> = None;
    for line in &archive_lines {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return SessionMergeResult {
                preview: base_preview(
                    false,
                    0,
                    None,
                    None,
                    "归档会话包含无法解析的记录，已停止合并。",
                ),
                contents: String::new(),
            };
        };
        if let Some(timestamp) = record_timestamp(&record) {
            archive_last = Some(archive_last.map_or(timestamp, |last| last.max(timestamp)));
        }
    }
    let Some(archive_last) = archive_last else {
        return SessionMergeResult {
            preview: base_preview(
                false,
                0,
                None,
                None,
                "归档会话没有可比较的时间戳，无法安全追加。",
            ),
            contents: String::new(),
        };
    };

    let archive_records = archive_lines
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut local_last: Option<DateTime<Utc>> = None;
    let mut last_appended_timestamp: Option<DateTime<Utc>> = None;
    let mut appended = Vec::new();
    for line in &local_lines {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return SessionMergeResult {
                preview: base_preview(
                    false,
                    0,
                    Some(archive_last),
                    local_last,
                    "本机会话包含无法解析的记录，已停止合并。",
                ),
                contents: String::new(),
            };
        };
        let Some(timestamp) = record_timestamp(&record) else {
            if !archive_records.contains(line.as_str()) {
                return SessionMergeResult {
                    preview: base_preview(
                        false,
                        0,
                        Some(archive_last),
                        local_last,
                        "本机续聊包含无法安全排序的无时间戳记录，已停止自动合并。",
                    ),
                    contents: String::new(),
                };
            }
            continue;
        };
        local_last = Some(local_last.map_or(timestamp, |last| last.max(timestamp)));
        if timestamp > archive_last && !archive_records.contains(line.as_str()) {
            if last_appended_timestamp.is_some_and(|last| timestamp < last) {
                return SessionMergeResult {
                    preview: base_preview(
                        false,
                        0,
                        Some(archive_last),
                        local_last,
                        "本机续聊的新增记录时间顺序倒退，已停止自动合并。",
                    ),
                    contents: String::new(),
                };
            }
            last_appended_timestamp = Some(timestamp);
            appended.push(line.clone());
        }
    }
    if appended.is_empty() {
        return SessionMergeResult {
            preview: base_preview(
                false,
                0,
                Some(archive_last),
                local_last,
                "本机没有时间晚于归档末尾的可追加记录。",
            ),
            contents: String::new(),
        };
    }

    let mut merged = archive_lines.clone();
    merged.extend(appended.iter().cloned());
    SessionMergeResult {
        preview: base_preview(
            true,
            appended.len(),
            Some(archive_last),
            local_last,
            "将以归档为基线，追加本机较新的记录。",
        ),
        contents: format!("{}\n", merged.join("\n")),
    }
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

fn database_tasks(home: &Path) -> HashMap<String, DatabaseTask> {
    let mut result = HashMap::new();
    let path = latest_state_database(home);
    let Ok(connection) =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return result;
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT id, COALESCE(title, ''), COALESCE(cwd, ''), COALESCE(first_user_message, ''), COALESCE(archived, 0), COALESCE(source, ''), COALESCE(model_provider, ''), COALESCE(model, ''), COALESCE(reasoning_effort, ''), COALESCE(sandbox_policy, ''), COALESCE(approval_mode, ''), COALESCE(cli_version, ''), COALESCE(thread_source, ''), COALESCE(agent_path, ''), COALESCE(agent_nickname, ''), COALESCE(agent_role, ''), COALESCE(memory_mode, ''), COALESCE(history_mode, '') FROM threads",
    ) else {
        return result;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            DatabaseTask {
                title: row.get::<_, String>(1)?,
                cwd: row.get::<_, String>(2)?,
                first_user_message: row.get::<_, String>(3)?,
                archived: row.get::<_, bool>(4)?,
                source: row.get::<_, String>(5)?,
                model_provider: row.get::<_, String>(6)?,
                model: row.get::<_, String>(7)?,
                reasoning_effort: row.get::<_, String>(8)?,
                sandbox_policy: row.get::<_, String>(9)?,
                approval_mode: row.get::<_, String>(10)?,
                cli_version: row.get::<_, String>(11)?,
                thread_source: row.get::<_, String>(12)?,
                agent_path: row.get::<_, String>(13)?,
                agent_nickname: row.get::<_, String>(14)?,
                agent_role: row.get::<_, String>(15)?,
                memory_mode: row.get::<_, String>(16)?,
                history_mode: row.get::<_, String>(17)?,
            },
        ))
    }) else {
        return result;
    };
    for row in rows.flatten() {
        result.insert(row.0, row.1);
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

fn catalog_visibility(
    catalog: &Option<HashMap<String, (String, String, bool)>>,
    task_id: &str,
) -> Option<bool> {
    catalog
        .as_ref()
        .and_then(|items| items.get(task_id).map(|item| item.2))
}

fn desktop_sidebar_visible(archived: bool, registered_in_sidebar: bool) -> bool {
    !archived && registered_in_sidebar
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
    let mut result = DesktopProjectState {
        project_order: value_array_strings(state.get("project-order")),
        projectless_threads: value_array_strings(state.get("projectless-thread-ids"))
            .into_iter()
            .collect(),
        ..Default::default()
    };
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

    Some(result)
}

fn path_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
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

fn path_segments(path: &str) -> Vec<String> {
    path.trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
        .collect()
}

fn usable_project_name(name: &str) -> Option<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.len() < 3 || is_generic_workspace_name(&name) {
        None
    } else {
        Some(name)
    }
}

fn desktop_project_name_matches_path(
    project: &DesktopProject,
    cwd: &str,
    inferred_path: &str,
) -> bool {
    let mut names = project
        .root_paths
        .iter()
        .filter_map(|path| usable_project_name(&path_name(path)))
        .collect::<HashSet<_>>();
    if let Some(name) = usable_project_name(&project.name) {
        names.insert(name);
    }
    if names.is_empty() {
        return false;
    }
    let segments = path_segments(cwd)
        .into_iter()
        .chain(path_segments(inferred_path))
        .collect::<HashSet<_>>();
    names.iter().any(|name| segments.contains(name))
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
        .or_else(|| {
            let candidates = state
                .projects
                .values()
                .filter(|project| desktop_project_name_matches_path(project, cwd, inferred_path))
                .collect::<Vec<_>>();
            match candidates.as_slice() {
                [project] => Some(*project),
                _ => {
                    let pinned = candidates
                        .iter()
                        .copied()
                        .filter(|project| project.pinned)
                        .collect::<Vec<_>>();
                    match pinned.as_slice() {
                        [project] => Some(*project),
                        _ => None,
                    }
                }
            }
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

fn prepend_unique_string(items: &mut Vec<Value>, value: &str) {
    if !items.iter().any(|item| item.as_str() == Some(value)) {
        items.insert(0, Value::String(value.to_string()));
    }
}

fn sort_tasks_by_project_order(tasks: &mut [Task], project_order: &[String]) {
    let positions = project_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<HashMap<_, _>>();
    tasks.sort_by_key(|task| {
        positions
            .get(task.project_key.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
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
        let cwd = task.cwd.trim().to_string();
        let has_project_folder = Path::new(&cwd).is_dir();
        let is_unbound = cwd.is_empty() || !has_project_folder;
        let (project_id, project_name, project_roots) = {
            // A missing historical cwd represents an unbound project.  Do not
            // attach it to an unrelated folder merely because the names match.
            let matched_project = has_project_folder
                .then(|| matching_desktop_project(&existing_state, &cwd, &cwd))
                .flatten();
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
                .unwrap_or_else(|| {
                    if cwd.is_empty() {
                        format!("unbound-{}", task.id)
                    } else {
                        stable_local_project_id(&cwd)
                    }
                });
            let project_name = matched_project
                .map(|project| project.name.clone())
                .unwrap_or_else(|| {
                    if is_unbound && cwd.is_empty() {
                        "未绑定项目".to_string()
                    } else {
                        path_name(&cwd)
                    }
                });
            let project_roots = matched_project
                .map(|project| project.root_paths.clone())
                .unwrap_or_else(|| {
                    if has_project_folder {
                        vec![cwd.clone()]
                    } else {
                        Vec::new()
                    }
                });
            (project_id, project_name, project_roots)
        };
        let project_already_exists = existing_state.projects.contains_key(&project_id);

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
                "cwd": if has_project_folder { cwd.clone() } else { String::new() },
                "pendingCoreUpdate": false
            }),
        );

        let project_order = array_mut(
            persisted
                .entry("project-order")
                .or_insert_with(|| Value::Array(Vec::new())),
        );
        // A recovered project no longer has an entry in Codex's previous order.
        // Put it at the top instead of silently appending it below every existing
        // project; existing projects retain their current position.
        if project_already_exists {
            push_unique_string(project_order, &project_id);
        } else {
            prepend_unique_string(project_order, &project_id);
        }

        if task.project_pinned && !project_already_exists {
            let pinned_projects = array_mut(
                persisted
                    .entry("pinned-project-ids")
                    .or_insert_with(|| Value::Array(Vec::new())),
            );
            push_unique_string(pinned_projects, &project_id);
        }

        if has_project_folder {
            let active_roots = array_mut(
                persisted
                    .entry("active-workspace-roots")
                    .or_insert_with(|| Value::Array(Vec::new())),
            );
            active_roots.retain(|item| match item.as_str() {
                Some(root) => Path::new(root).is_dir(),
                None => true,
            });
            push_unique_string(active_roots, &cwd);
        }

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
        if has_project_folder {
            workspace_hints.insert(task.id.clone(), Value::String(cwd.clone()));
        } else {
            workspace_hints.remove(&task.id);
        }

        let writable_roots = object_mut(
            root.entry("thread-writable-roots")
                .or_insert_with(|| Value::Object(Map::new())),
        );
        if has_project_folder {
            let roots = array_mut(
                writable_roots
                    .entry(task.id.clone())
                    .or_insert_with(|| Value::Array(Vec::new())),
            );
            roots.retain(|item| match item.as_str() {
                Some(root) => root == cwd || Path::new(root).is_dir(),
                None => true,
            });
            push_unique_string(roots, &cwd);
        } else {
            writable_roots.remove(&task.id);
        }
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

fn session_file_paths(home: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for session_root in [home.join("sessions"), home.join("archived_sessions")] {
        if session_root.exists() {
            for entry in WalkDir::new(session_root).into_iter().flatten() {
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                {
                    files.push(entry.path().to_path_buf());
                }
            }
        }
    }
    files
}

fn scan_preview_task(home: &Path, path: &Path) -> Option<Task> {
    let mut task = session_details(path).ok()?;
    if task.id.is_empty() {
        return None;
    }
    task.archived = path.starts_with(home.join("archived_sessions"));
    task.browser_file = home
        .join("browser")
        .join("sessions")
        .join(format!("{}.toml", task.id));
    (
        task.project_key,
        task.project_name,
        task.project_path,
        task.project_exists,
    ) = infer_project(&task);
    normalize_task_title(&mut task);
    if task.title.is_empty() {
        task.title = truncate(&task.first_user_message, 96);
    }
    if task.title.is_empty() {
        task.title = format!("未命名任务 {}", &task.id[..task.id.len().min(8)]);
    }
    Some(task)
}

fn session_tasks(home: &Path) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut seen_task_ids = HashSet::new();
    for path in session_file_paths(home) {
        let Ok(mut task) = session_details(&path) else {
            continue;
        };
        if task.id.is_empty() || !seen_task_ids.insert(task.id.clone()) {
            continue;
        }
        // Prefer the active copy when the same task also has an archived residue.
        task.archived = path.starts_with(home.join("archived_sessions"));
        task.browser_file = home
            .join("browser")
            .join("sessions")
            .join(format!("{}.toml", task.id));
        tasks.push(task);
    }
    tasks
}

fn enrich_local_tasks_with_title_ids(
    home: &Path,
    raw_tasks: Vec<Task>,
) -> Result<(Vec<Task>, HashSet<String>), String> {
    let index = read_index(home);
    let history_messages = history_first_messages(home);
    let database = database_tasks(home);
    let catalog = catalog_tasks(home);
    let affected_title_ids = bad_title_ids_from_metadata(&index, &database, &catalog);
    let desktop_projects = read_desktop_project_state(home);
    let mut tasks = Vec::new();
    for mut task in raw_tasks {
        let indexed = index.get(&task.id);
        let database_task = database.get(&task.id);
        let catalog_task = catalog.as_ref().and_then(|items| items.get(&task.id));
        task.title = indexed
            .and_then(|item| item.get("thread_name"))
            .and_then(Value::as_str)
            .filter(|title| !is_bad_title(title))
            .map(str::to_string)
            .or_else(|| {
                catalog_task
                    .filter(|item| !is_bad_title(&item.0))
                    .map(|item| item.0.clone())
            })
            .or_else(|| database_task.map(|item| item.title.clone()))
            .filter(|title| !is_bad_title(title))
            .or_else(|| history_messages.get(&task.id).cloned())
            .unwrap_or_else(|| truncate(&task.first_user_message, 96));
        normalize_task_title(&mut task);
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
                .filter(|item| !item.1.is_empty())
                .map(|item| item.1.clone())
                .or_else(|| database_task.map(|item| item.cwd.clone()))
                .unwrap_or_default();
        } else if let Some(catalog_cwd) = catalog_task
            .filter(|item| !item.1.is_empty())
            .map(|item| item.1.clone())
        {
            task.cwd = catalog_cwd;
        }
        if let Some(database_task) = database_task {
            if task.source.is_empty() {
                task.source = database_task.source.clone();
            }
            if task.model_provider.is_empty() {
                task.model_provider = database_task.model_provider.clone();
            }
            if task.model.is_empty() {
                task.model = database_task.model.clone();
            }
            if task.reasoning_effort.is_empty() {
                task.reasoning_effort = database_task.reasoning_effort.clone();
            }
            if task.sandbox_policy.is_empty() {
                task.sandbox_policy = database_task.sandbox_policy.clone();
            }
            if task.approval_mode.is_empty() {
                task.approval_mode = database_task.approval_mode.clone();
            }
            if task.cli_version.is_empty() {
                task.cli_version = database_task.cli_version.clone();
            }
            if task.thread_source.is_empty() {
                task.thread_source = database_task.thread_source.clone();
            }
            if task.agent_path.is_empty() {
                task.agent_path = database_task.agent_path.clone();
            }
            if task.agent_nickname.is_empty() {
                task.agent_nickname = database_task.agent_nickname.clone();
            }
            if task.agent_role.is_empty() {
                task.agent_role = database_task.agent_role.clone();
            }
            if task.memory_mode.is_empty() {
                task.memory_mode = database_task.memory_mode.clone();
            }
            if task.history_mode.is_empty() {
                task.history_mode = database_task.history_mode.clone();
            }
        }
        task.archived = task.archived || database_task.map(|item| item.archived).unwrap_or(false);
        let catalog_visible = catalog_visibility(&catalog, &task.id);
        task.codex_visible = catalog_visible.unwrap_or(true);
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
                    task.codex_visible = desktop_sidebar_visible(task.archived, true);
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
                    // Codex keeps a client-thread entry after a project is removed from
                    // the sidebar. The dangling project assignment is the authoritative
                    // signal in this case, not the stale client-thread entry.
                    task.codex_visible = false;
                }
            } else if let Some(project) =
                matching_desktop_project(project_state, &task.cwd, &task.project_path)
            {
                apply_desktop_project(&mut task, project);
                task.codex_visible = desktop_sidebar_visible(task.archived, true);
            } else {
                task.codex_visible = desktop_sidebar_visible(
                    task.archived,
                    project_state.projectless_threads.contains(&task.id),
                );
            }
        }
        tasks.push(task);
    }
    tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok((tasks, affected_title_ids))
}

fn list_local_tasks_with_title_ids() -> Result<(Vec<Task>, HashSet<String>), String> {
    let home = codex_home();
    enrich_local_tasks_with_title_ids(&home, session_tasks(&home))
}

fn list_local_tasks() -> Result<Vec<Task>, String> {
    Ok(list_local_tasks_with_title_ids()?.0)
}

fn archive_path(id: &str, file: &str) -> String {
    format!("tasks/{id}/{file}")
}

fn active_session_copy_path(home: &Path, task: &Task) -> PathBuf {
    let date = parse_time(if task.created_at.is_empty() {
        &task.updated_at
    } else {
        &task.created_at
    });
    home.join("sessions")
        .join(date.year().to_string())
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}", date.day()))
        .join(format!(
            "rollout-{}-{}.jsonl",
            date.format("%Y-%m-%dT%H-%M-%S"),
            task.id
        ))
}

fn is_safe_task_id(id: &str) -> bool {
    id.len() >= 8
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn session_meta_id<R: BufRead>(reader: R) -> Result<String, String> {
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let id = record
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "压缩包会话缺少 ID".to_string())?;
        return Ok(id.to_string());
    }
    Err("压缩包会话缺少 session_meta".to_string())
}

fn session_meta_forked_from_id<R: BufRead>(reader: R) -> Result<String, String> {
    for line in reader.lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        return Ok(record
            .get("payload")
            .and_then(|payload| payload.get("forked_from_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or_default()
            .to_string());
    }
    Err("压缩包会话缺少 session_meta".to_string())
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
            !is_safe_task_id(&task.task.id)
                || (!task.task.forked_from_id.is_empty()
                    && !is_safe_task_id(&task.task.forked_from_id))
                || !task
                    .session_file
                    .starts_with(&format!("tasks/{}/", task.task.id))
                || task
                    .browser_file
                    .as_deref()
                    .is_some_and(|file| !file.starts_with(&format!("tasks/{}/", task.task.id)))
        })
    {
        return Err("这不是有效的 Codex 会话迁移压缩包".to_string());
    }
    let mut ids = HashSet::new();
    for task in &manifest.tasks {
        if !ids.insert(task.task.id.as_str()) {
            return Err("压缩包包含重复会话 ID".to_string());
        }
        let session = zip
            .by_name(&task.session_file)
            .map_err(|_| "压缩包缺少会话文件".to_string())?;
        let session_id = session_meta_id(BufReader::new(session))?;
        if session_id != task.task.id {
            return Err("压缩包会话 ID 与 manifest 不一致".to_string());
        }
        let session = zip
            .by_name(&task.session_file)
            .map_err(|_| "压缩包缺少会话文件".to_string())?;
        let session_forked_from_id = session_meta_forked_from_id(BufReader::new(session))?;
        if !task.task.forked_from_id.is_empty()
            && task.task.forked_from_id != session_forked_from_id
        {
            return Err("压缩包会话关系与 manifest 不一致".to_string());
        }
        if let Some(browser_file) = &task.browser_file {
            zip.by_name(browser_file)
                .map_err(|_| "压缩包缺少浏览器配置".to_string())?;
        }
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
    let name = path_name(cwd);
    let home = dirs::home_dir().unwrap_or_default();
    let fallback = home.join("work").join(&name);
    for candidate in [
        fallback.clone(),
        home.join("Projects").join(&name),
        home.join("Documents").join(&name),
    ] {
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }
    if !name.is_empty() {
        return fallback.to_string_lossy().to_string();
    }
    cwd.to_string()
}

fn resolved_import_cwd(source_cwd: &str, target_cwd: Option<&str>, adapt_paths: bool) -> String {
    if source_cwd.trim().is_empty() {
        return String::new();
    }
    // Worktrees are owned by Codex/Git. A historical worktree is restored as
    // unbound when it is absent locally; this utility never creates or binds it.
    if is_codex_worktree(source_cwd) && !Path::new(source_cwd).is_dir() {
        return String::new();
    }
    if let Some(target) = target_cwd.map(str::trim).filter(|path| !path.is_empty()) {
        return if Path::new(target).is_dir() {
            target.to_string()
        } else {
            String::new()
        };
    }
    if !adapt_paths {
        return if Path::new(source_cwd).is_dir() {
            source_cwd.to_string()
        } else {
            String::new()
        };
    }
    let resolved = resolve_local_cwd(source_cwd);
    if Path::new(&resolved).is_dir() {
        resolved
    } else {
        String::new()
    }
}

fn is_path_field(name: &str) -> bool {
    matches!(
        name,
        "cwd"
            | "root"
            | "rootPath"
            | "rootPaths"
            | "workspaceRoot"
            | "workspace_root"
            | "writableRoots"
            | "writable_roots"
    )
}

fn replace_path_value(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => *text = text.replace(from, to),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_path_value(item, from, to)),
        Value::Object(items) => items.iter_mut().for_each(|(name, item)| {
            if is_path_field(name) {
                replace_path_value(item, from, to);
            }
        }),
        _ => {}
    }
}

fn rewrite_path_fields(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| rewrite_path_fields(item, from, to)),
        Value::Object(items) => items.iter_mut().for_each(|(name, item)| {
            if is_path_field(name) {
                replace_path_value(item, from, to);
            } else {
                rewrite_path_fields(item, from, to);
            }
        }),
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
                rewrite_path_fields(&mut value, from, to);
                serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
            }
            Err(_) => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn rewrite_session_meta_cwd(contents: &str, to: &str) -> String {
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

fn normalize_task_to_codex_model(task: &mut Task, settings: &CodexModelSettings) {
    task.model_provider = settings.provider.clone();
    task.model = settings.model.clone();
    task.reasoning_effort = settings.reasoning_effort.clone();
}

fn rewrite_session_model_context(contents: &str, settings: &CodexModelSettings) -> String {
    contents
        .lines()
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(mut value) => {
                if value.get("type").and_then(Value::as_str) == Some("session_meta") {
                    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert(
                            "model_provider".to_string(),
                            Value::String(settings.provider.clone()),
                        );
                        payload.insert("model".to_string(), Value::String(settings.model.clone()));
                        payload.insert(
                            "reasoning_effort".to_string(),
                            Value::String(settings.reasoning_effort.clone()),
                        );
                    }
                }

                if value.get("type").and_then(Value::as_str) == Some("turn_context") {
                    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert("model".to_string(), Value::String(settings.model.clone()));
                        payload.insert(
                            "effort".to_string(),
                            Value::String(settings.reasoning_effort.clone()),
                        );
                        payload.insert(
                            "reasoning_effort".to_string(),
                            Value::String(settings.reasoning_effort.clone()),
                        );
                        if let Some(settings_value) = payload
                            .get_mut("collaboration_mode")
                            .and_then(Value::as_object_mut)
                            .and_then(|mode| mode.get_mut("settings"))
                            .and_then(Value::as_object_mut)
                        {
                            settings_value
                                .insert("model".to_string(), Value::String(settings.model.clone()));
                            settings_value.insert(
                                "reasoning_effort".to_string(),
                                Value::String(settings.reasoning_effort.clone()),
                            );
                        }
                    }
                }

                if value
                    .get("payload")
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str)
                    == Some("thread_settings_applied")
                {
                    if let Some(thread_settings) = value
                        .get_mut("payload")
                        .and_then(Value::as_object_mut)
                        .and_then(|payload| payload.get_mut("thread_settings"))
                        .and_then(Value::as_object_mut)
                    {
                        thread_settings.insert(
                            "model_provider_id".to_string(),
                            Value::String(settings.provider.clone()),
                        );
                        thread_settings.insert(
                            "model_provider".to_string(),
                            Value::String(settings.provider.clone()),
                        );
                        thread_settings
                            .insert("model".to_string(), Value::String(settings.model.clone()));
                        thread_settings.insert(
                            "reasoning_effort".to_string(),
                            Value::String(settings.reasoning_effort.clone()),
                        );
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
    let replacements = tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect::<HashMap<_, _>>();
    let mut written = HashSet::new();
    let mut lines = Vec::new();
    for line in fs::read_to_string(&path).unwrap_or_default().lines() {
        let Ok(mut value) = serde_json::from_str::<Value>(line) else {
            lines.push(line.to_string());
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            lines.push(line.to_string());
            continue;
        };
        if let Some(replacement) = replacements.get(id) {
            if written.insert(id.to_string()) {
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "thread_name".to_string(),
                        Value::String(replacement.title.clone()),
                    );
                    object.insert(
                        "updated_at".to_string(),
                        Value::String(replacement.updated_at.clone()),
                    );
                }
                lines.push(value.to_string());
            }
        } else {
            lines.push(line.to_string());
        }
    }
    for task in tasks {
        if written.insert(task.id.clone()) {
            lines.push(
                serde_json::json!({"id": task.id, "thread_name": task.title, "updated_at": task.updated_at})
                    .to_string(),
            );
        }
    }
    fs::write(path, format!("{}\n", lines.join("\n"))).map_err(|error| error.to_string())
}

fn verify_registered_threads(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let state_exists = latest_state_database(home).exists();
    let catalog = catalog_tasks(home);
    let catalog_available = catalog.is_some();
    if !state_exists && !catalog_available {
        return Err("未找到 Codex 的任务数据库，无法确认恢复结果".to_string());
    }
    let database = database_tasks(home);
    for task in tasks {
        if state_exists && !database.get(&task.id).is_some_and(|item| !item.archived) {
            return Err(format!("任务 {} 未成功登记到 Codex 状态库", task.id));
        }
        if catalog_available && !catalog_visibility(&catalog, &task.id).unwrap_or(false) {
            return Err(format!("任务 {} 未成功登记到 Codex 侧边栏目录", task.id));
        }
    }
    Ok(())
}

fn sqlite_sidecar(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}-{suffix}", path.display()))
}

fn codex_state_paths(home: &Path) -> Vec<PathBuf> {
    let state = latest_state_database(home);
    let catalog = home.join("sqlite").join("codex-dev.db");
    vec![
        home.join("session_index.jsonl"),
        state.clone(),
        sqlite_sidecar(&state, "wal"),
        sqlite_sidecar(&state, "shm"),
        catalog.clone(),
        sqlite_sidecar(&catalog, "wal"),
        sqlite_sidecar(&catalog, "shm"),
        home.join(".codex-global-state.json"),
    ]
}

fn checkpoint_sqlite_database(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| format!("无法检查点数据库 {}: {error}", path.display()))
}

struct ImportTransaction {
    backups: Vec<String>,
    backed_paths: HashSet<PathBuf>,
    created_files: Vec<PathBuf>,
    state_files: Vec<(PathBuf, bool)>,
    committed: bool,
}

impl ImportTransaction {
    fn new(home: &Path) -> Self {
        let state_files = codex_state_paths(home)
            .into_iter()
            .map(|path| {
                let exists = path.exists();
                (path, exists)
            })
            .collect();
        Self {
            backups: Vec::new(),
            backed_paths: HashSet::new(),
            created_files: Vec::new(),
            state_files,
            committed: false,
        }
    }

    fn backup(&mut self, path: &Path, stamp: &str) -> Result<(), String> {
        if !path.exists() || !self.backed_paths.insert(path.to_path_buf()) {
            return Ok(());
        }
        let target = PathBuf::from(format!("{}{}{}", path.display(), LOCAL_SNAPSHOT_MARKER, stamp));
        fs::copy(path, &target).map_err(|error| format!("无法备份 {}: {error}", path.display()))?;
        self.backups.push(target.to_string_lossy().to_string());
        Ok(())
    }

    fn write_file(
        &mut self,
        path: &Path,
        contents: impl AsRef<[u8]>,
        stamp: &str,
    ) -> Result<(), String> {
        let existed = path.exists();
        self.backup(path, stamp)?;
        if !existed {
            self.created_files.push(path.to_path_buf());
        }
        fs::write(path, contents).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn commit(mut self) -> Vec<String> {
        self.committed = true;
        std::mem::take(&mut self.backups)
    }
}

impl Drop for ImportTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.created_files.iter().rev() {
            fs::remove_file(path).ok();
        }
        for backup in self.backups.iter().rev() {
            if let Some((original, _)) = backup.rsplit_once(LOCAL_SNAPSHOT_MARKER) {
                fs::copy(backup, original).ok();
            }
        }
        for (path, existed) in &self.state_files {
            if !existed {
                fs::remove_file(path).ok();
            }
        }
    }
}

fn receipt_path(home: &Path) -> PathBuf {
    home.join("session-transfer").join("receipts.jsonl")
}

const MAX_CURRENT_RECEIPT_BYTES: u64 = 1024 * 1024;
const MAX_ROTATED_RECEIPTS: usize = 8;

fn prune_rotated_operation_receipts(parent: &Path) -> Result<(), String> {
    let mut archives = fs::read_dir(parent)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("receipts-") || !name.ends_with(".jsonl") {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    archives.sort_by_key(|entry| entry.0);
    let excess = archives.len().saturating_sub(MAX_ROTATED_RECEIPTS);
    for (_, path) in archives.into_iter().take(excess) {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn append_operation_receipt(home: &Path, kind: &str, result: &Value) -> Result<String, String> {
    let path = receipt_path(home);
    let parent = path
        .parent()
        .ok_or_else(|| "无法创建维护回执目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if fs::metadata(&path)
        .map(|metadata| metadata.len() >= MAX_CURRENT_RECEIPT_BYTES)
        .unwrap_or(false)
    {
        let stamp = Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
        let mut suffix = 0usize;
        let mut rotated = parent.join(format!("receipts-{stamp}.jsonl"));
        while rotated.exists() {
            suffix += 1;
            rotated = parent.join(format!("receipts-{stamp}-{suffix}.jsonl"));
        }
        fs::rename(&path, rotated).map_err(|error| error.to_string())?;
        prune_rotated_operation_receipts(parent)?;
    }
    let receipt = serde_json::json!({
        "createdAt": Utc::now().to_rfc3339(),
        "kind": kind,
        "result": result,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&receipt).map_err(|error| error.to_string())?
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationIntegrity {
    name: String,
    status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationIssue {
    id: String,
    title: String,
    code: String,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskLibraryValidation {
    checked_at: String,
    task_count: usize,
    issue_count: usize,
    healthy: bool,
    integrity: Vec<ValidationIntegrity>,
    issues: Vec<ValidationIssue>,
}

fn sqlite_integrity(path: &Path, name: &str) -> ValidationIntegrity {
    if !path.exists() {
        return ValidationIntegrity {
            name: name.to_string(),
            status: "missing".to_string(),
        };
    }
    let status = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|connection| {
            connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        })
        .unwrap_or_else(|error| format!("error: {error}"));
    ValidationIntegrity {
        name: name.to_string(),
        status,
    }
}

#[tauri::command]
async fn validate_task_library() -> Result<TaskLibraryValidation, String> {
    tauri::async_runtime::spawn_blocking(validate_task_library_blocking)
        .await
        .map_err(|error| error.to_string())?
}

fn validate_task_library_blocking() -> Result<TaskLibraryValidation, String> {
    let home = codex_home();
    let tasks = list_local_tasks()?;
    let index = read_index(&home);
    let database = database_tasks(&home);
    let session_ids = tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<HashSet<_>>();
    let mut issues = Vec::new();

    for task in &tasks {
        if !index.contains_key(&task.id) {
            issues.push(ValidationIssue {
                id: task.id.clone(),
                title: task.title.clone(),
                code: "index_missing".to_string(),
                detail: "会话文件存在，但 session_index.jsonl 没有对应任务。".to_string(),
            });
        }
        if !database.contains_key(&task.id) {
            issues.push(ValidationIssue {
                id: task.id.clone(),
                title: task.title.clone(),
                code: "state_missing".to_string(),
                detail: "会话文件存在，但任务状态数据库没有对应记录。".to_string(),
            });
        }
    }

    let mut missing_session_ids = HashSet::new();
    for (id, item) in &index {
        if !session_ids.contains(id) && missing_session_ids.insert(id.clone()) {
            issues.push(ValidationIssue {
                id: id.clone(),
                title: item
                    .get("thread_name")
                    .and_then(Value::as_str)
                    .unwrap_or("未命名任务")
                    .to_string(),
                code: "session_missing".to_string(),
                detail: "索引存在，但没有找到对应的本地会话文件。".to_string(),
            });
        }
    }
    for (id, task) in &database {
        if !session_ids.contains(id) && missing_session_ids.insert(id.clone()) {
            issues.push(ValidationIssue {
                id: id.clone(),
                title: task.title.clone(),
                code: "session_missing".to_string(),
                detail: "任务状态存在，但没有找到对应的本地会话文件。".to_string(),
            });
        }
    }

    let integrity = vec![
        {
            let state = latest_state_database(&home);
            sqlite_integrity(
                &state,
                state
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("state.sqlite"),
            )
        },
        sqlite_integrity(&home.join("sqlite").join("codex-dev.db"), "codex-dev.db"),
    ];
    let healthy = issues.is_empty() && integrity.iter().all(|item| item.status == "ok");
    let issue_count = issues.len();
    issues.truncate(50);
    Ok(TaskLibraryValidation {
        checked_at: Utc::now().to_rfc3339(),
        task_count: tasks.len(),
        issue_count,
        healthy,
        integrity,
        issues,
    })
}

#[tauri::command]
fn list_operation_receipts() -> Result<Vec<Value>, String> {
    let path = receipt_path(&codex_home());
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    Ok(contents
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .take(12)
        .collect())
}

#[tauri::command]
fn get_operation_receipts_directory() -> String {
    receipt_path(&codex_home())
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn is_local_snapshot_file(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(LOCAL_SNAPSHOT_MARKER))
}

fn local_snapshots(home: &Path) -> Result<Vec<LocalSnapshot>, String> {
    if !home.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::new();
    for entry in WalkDir::new(home)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !is_local_snapshot_file(path) {
            continue;
        }
        let metadata = fs::metadata(path)
            .map_err(|error| format!("无法读取快照 {}: {error}", path.display()))?;
        let modified_at = metadata
            .modified()
            .map(DateTime::<Utc>::from)
            .unwrap_or_else(|_| Utc::now())
            .to_rfc3339();
        snapshots.push(LocalSnapshot {
            path: path.to_string_lossy().to_string(),
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("未命名快照")
                .to_string(),
            size: metadata.len(),
            modified_at,
        });
    }
    snapshots.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    Ok(snapshots)
}

fn validated_local_snapshot_path(home: &Path, requested_path: &str) -> Result<PathBuf, String> {
    let canonical_home =
        fs::canonicalize(home).map_err(|error| format!("无法访问 Codex 数据目录: {error}"))?;
    let candidate = fs::canonicalize(Path::new(requested_path))
        .map_err(|_| "快照不存在或已被删除。".to_string())?;
    if !candidate.starts_with(&canonical_home) || !is_local_snapshot_file(&candidate) {
        return Err("只能删除当前 Codex 数据目录中由本工具创建的快照。".to_string());
    }
    Ok(candidate)
}

fn delete_local_snapshots_blocking(
    snapshot_paths: Vec<String>,
) -> Result<SnapshotDeletionResult, String> {
    if snapshot_paths.is_empty() {
        return Ok(SnapshotDeletionResult {
            deleted_count: 0,
            reclaimed_bytes: 0,
        });
    }
    let home = codex_home();
    let _write_guard = acquire_local_write_operation(&home)?;
    let snapshots = snapshot_paths
        .iter()
        .map(|path| validated_local_snapshot_path(&home, path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut reclaimed_bytes = 0;
    for snapshot in &snapshots {
        reclaimed_bytes += fs::metadata(snapshot)
            .map_err(|error| format!("无法读取快照 {}: {error}", snapshot.display()))?
            .len();
    }
    for snapshot in &snapshots {
        fs::remove_file(snapshot)
            .map_err(|error| format!("无法删除快照 {}: {error}", snapshot.display()))?;
    }
    Ok(SnapshotDeletionResult {
        deleted_count: snapshots.len(),
        reclaimed_bytes,
    })
}

#[tauri::command]
async fn list_local_snapshots() -> Result<Vec<LocalSnapshot>, String> {
    tauri::async_runtime::spawn_blocking(|| local_snapshots(&codex_home()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn delete_local_snapshots(
    snapshot_paths: Vec<String>,
) -> Result<SnapshotDeletionResult, String> {
    tauri::async_runtime::spawn_blocking(move || delete_local_snapshots_blocking(snapshot_paths))
        .await
        .map_err(|error| error.to_string())?
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    }
}

fn optional_text(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn replace_if_present(target: &mut String, source: &str) {
    if !source.trim().is_empty() {
        *target = source.trim().to_string();
    }
}

fn register_threads(home: &Path, tasks: &[Task]) -> Result<(), String> {
    let state = latest_state_database(home);
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
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, cwd, title, sandbox_policy, approval_mode, git_branch, git_origin_url, first_user_message, memory_mode, preview, recency_at, recency_at_ms, history_mode, has_user_event, archived, archived_at, cli_version, model, reasoning_effort, thread_source, agent_path, agent_nickname, agent_role, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?24, ?25, ?9, ?10, ?11, ?18, ?12, ?4, ?13, ?19, 1, 0, NULL, ?14, ?15, ?16, ?17, ?20, ?21, ?22, ?23, ?13) ON CONFLICT(id) DO UPDATE SET rollout_path=excluded.rollout_path, updated_at=excluded.updated_at, source=excluded.source, model_provider=excluded.model_provider, cwd=excluded.cwd, title=excluded.title, sandbox_policy=excluded.sandbox_policy, approval_mode=excluded.approval_mode, git_branch=excluded.git_branch, git_origin_url=excluded.git_origin_url, first_user_message=excluded.first_user_message, preview=excluded.preview, recency_at=excluded.recency_at, recency_at_ms=excluded.recency_at_ms, history_mode=excluded.history_mode, cli_version=excluded.cli_version, model=excluded.model, reasoning_effort=excluded.reasoning_effort, thread_source=excluded.thread_source, agent_path=excluded.agent_path, agent_nickname=excluded.agent_nickname, agent_role=excluded.agent_role, memory_mode=excluded.memory_mode, created_at_ms=excluded.created_at_ms, updated_at_ms=excluded.updated_at_ms, has_user_event=1, archived=0, archived_at=NULL",
                params![
                    task.id,
                    task.file_path.to_string_lossy(),
                    parse_time(&task.created_at).timestamp(),
                    parse_time(&task.updated_at).timestamp(),
                    non_empty_or(&task.source, "vscode"),
                    non_empty_or(&task.model_provider, "openai"),
                    task.cwd,
                    task.title,
                    task.git_branch,
                    task.git_origin_url,
                    task.first_user_message,
                    task.preview,
                    parse_time(&task.updated_at).timestamp_millis(),
                    task.cli_version,
                    optional_text(&task.model),
                    optional_text(&task.reasoning_effort),
                    optional_text(&task.thread_source),
                    non_empty_or(&task.memory_mode, "enabled"),
                    non_empty_or(&task.history_mode, "legacy"),
                    optional_text(&task.agent_path),
                    optional_text(&task.agent_nickname),
                    optional_text(&task.agent_role),
                    parse_time(&task.created_at).timestamp_millis(),
                    non_empty_or(&task.sandbox_policy, "{\"type\":\"disabled\"}"),
                    non_empty_or(&task.approval_mode, "never"),
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
            "UPDATE local_thread_catalog_sync_state SET observation_sequence = MAX(observation_sequence, ?1), initial_build_complete = 1 WHERE host_id = 'local'",
            params![current + tasks.len() as i64],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

fn bad_title_ids_from_metadata(
    index: &HashMap<String, Value>,
    database: &HashMap<String, DatabaseTask>,
    catalog: &Option<HashMap<String, (String, String, bool)>>,
) -> HashSet<String> {
    let mut affected_ids = HashSet::new();
    for (id, item) in index {
        if item
            .get("thread_name")
            .and_then(Value::as_str)
            .is_some_and(is_bad_title)
        {
            affected_ids.insert(id.clone());
        }
    }
    for (id, task) in database {
        if is_bad_title(&task.title) || is_codex_context_text(&task.first_user_message) {
            affected_ids.insert(id.clone());
        }
    }
    if let Some(catalog) = catalog {
        for (id, item) in catalog {
            if is_bad_title(&item.0) {
                affected_ids.insert(id.clone());
            }
        }
    }
    affected_ids
}

fn bad_title_ids(home: &Path) -> HashSet<String> {
    let index = read_index(home);
    let database = database_tasks(home);
    let catalog = catalog_tasks(home);
    bad_title_ids_from_metadata(&index, &database, &catalog)
}

fn task_health_issues(task: &Task, _affected_title_ids: &HashSet<String>) -> Vec<TaskHealthIssue> {
    let mut issues = Vec::new();
    if !task.file_path.is_file() {
        issues.push(TaskHealthIssue {
            code: "session_file_missing".to_string(),
            level: "manual".to_string(),
            title: "会话文件不可读取".to_string(),
            detail: "找不到对应的本地 JSONL 会话文件，无法安全写入或重新登记。".to_string(),
            recommended_action: String::new(),
        });
    }
    if task.archived || !task.codex_visible {
        issues.push(TaskHealthIssue {
            code: "not_registered".to_string(),
            level: "repairable".to_string(),
            title: "任务未在 Codex 侧栏显示".to_string(),
            detail: "会话文件仍在本机；可以重新登记任务，并取消归档或隐藏状态。".to_string(),
            recommended_action: "reregister".to_string(),
        });
    }
    if !task.project_exists {
        issues.push(TaskHealthIssue {
            code: "unbound_project".to_string(),
            level: "info".to_string(),
            title: "历史项目路径不存在".to_string(),
            detail: "任务会保留为未绑定项目；本次修复不会创建同名文件夹或绑定错误路径。"
                .to_string(),
            recommended_action: "keep_unbound".to_string(),
        });
    }
    issues
}

fn task_health_item(task: &Task, affected_title_ids: &HashSet<String>) -> TaskHealthItem {
    let issues = task_health_issues(task, affected_title_ids);
    let mut safe_actions = Vec::new();
    for issue in &issues {
        if issue.level == "repairable" && !issue.recommended_action.is_empty() {
            safe_actions.push(issue.recommended_action.clone());
        }
    }
    safe_actions.sort();
    safe_actions.dedup();
    let requires_manual_review = issues.iter().any(|issue| issue.level == "manual");
    TaskHealthItem {
        id: task.id.clone(),
        title: task.title.clone(),
        cwd: task.cwd.clone(),
        issues,
        safe_actions,
        requires_manual_review,
    }
}

fn build_repair_plan_for(task_ids: &[String]) -> Result<RepairPlan, String> {
    let requested: HashSet<_> = task_ids.iter().cloned().collect();
    if requested.is_empty() {
        return Err("请至少选择一个需要修复的任务".to_string());
    }
    let home = codex_home();
    let affected_title_ids = bad_title_ids(&home);
    let tasks = list_local_tasks()?;
    let mut items = Vec::new();
    for task in tasks
        .into_iter()
        .filter(|task| requested.contains(&task.id))
    {
        let health = task_health_item(&task, &affected_title_ids);
        let can_apply = !health.requires_manual_review && !health.safe_actions.is_empty();
        let reason = if health.requires_manual_review {
            "会话文件不完整，需人工处理；本工具不会猜测性写入。".to_string()
        } else if health.safe_actions.is_empty() {
            "当前任务不需要安全修复。".to_string()
        } else {
            "执行前将创建 Codex 本地状态快照。".to_string()
        };
        items.push(RepairPlanItem {
            id: task.id,
            title: task.title,
            cwd: task.cwd,
            actions: health.safe_actions,
            can_apply,
            reason,
        });
    }
    let missing_count = requested.len().saturating_sub(items.len());
    for id in requested {
        if !items.iter().any(|item| item.id == id) {
            items.push(RepairPlanItem {
                id,
                title: "未找到本地任务".to_string(),
                cwd: String::new(),
                actions: Vec::new(),
                can_apply: false,
                reason: "本机没有找到可对应的会话文件；不会执行写入。".to_string(),
            });
        }
    }
    Ok(RepairPlan {
        actionable_count: items.iter().filter(|item| item.can_apply).count(),
        manual_review_count: items.iter().filter(|item| !item.can_apply).count(),
        snapshot_note: if missing_count > 0 {
            "可执行任务会先备份 Codex 数据库、索引和全局状态；缺失任务会被跳过。".to_string()
        } else {
            "执行前会备份 Codex 数据库、索引和全局状态。".to_string()
        },
        items,
    })
}

#[tauri::command]
fn get_task_health() -> Result<TaskHealthReport, String> {
    let home = codex_home();
    let affected_title_ids = bad_title_ids(&home);
    let tasks = list_local_tasks()?;
    let items = tasks
        .iter()
        .map(|task| task_health_item(task, &affected_title_ids))
        .collect::<Vec<_>>();
    Ok(TaskHealthReport {
        summary: TaskHealthSummary {
            healthy_count: items.iter().filter(|item| item.issues.is_empty()).count(),
            reregister_count: items
                .iter()
                .filter(|item| {
                    item.safe_actions
                        .iter()
                        .any(|action| action == "reregister")
                })
                .count(),
            title_repair_count: items
                .iter()
                .filter(|item| {
                    item.safe_actions
                        .iter()
                        .any(|action| action == "repair_title")
                })
                .count(),
            manual_review_count: items
                .iter()
                .filter(|item| item.requires_manual_review)
                .count(),
            unbound_project_count: items
                .iter()
                .filter(|item| {
                    item.issues
                        .iter()
                        .any(|issue| issue.code == "unbound_project")
                })
                .count(),
        },
        tasks: items,
    })
}

fn task_health_report(tasks: &[Task], affected_title_ids: &HashSet<String>) -> TaskHealthReport {
    let items = tasks
        .iter()
        .map(|task| task_health_item(task, affected_title_ids))
        .collect::<Vec<_>>();
    TaskHealthReport {
        summary: TaskHealthSummary {
            healthy_count: items.iter().filter(|item| item.issues.is_empty()).count(),
            reregister_count: items
                .iter()
                .filter(|item| {
                    item.safe_actions
                        .iter()
                        .any(|action| action == "reregister")
                })
                .count(),
            title_repair_count: items
                .iter()
                .filter(|item| {
                    item.safe_actions
                        .iter()
                        .any(|action| action == "repair_title")
                })
                .count(),
            manual_review_count: items
                .iter()
                .filter(|item| item.requires_manual_review)
                .count(),
            unbound_project_count: items
                .iter()
                .filter(|item| {
                    item.issues
                        .iter()
                        .any(|issue| issue.code == "unbound_project")
                })
                .count(),
        },
        tasks: items,
    }
}

fn emit_scan_event(app: &tauri::AppHandle, payload: Value) {
    let _ = app.emit("task-scan-progress", payload);
}

#[tauri::command]
fn start_task_scan(
    app: tauri::AppHandle,
    run_id: String,
    resume_token: Option<String>,
) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err("扫描标识无效".to_string());
    }
    clear_background_job(&run_id);
    std::thread::spawn(move || {
        let started = Instant::now();
        let home = codex_home();
        let resume_token = resume_token.filter(|token| !token.trim().is_empty());
        let continuation_token = resume_token.clone().unwrap_or_else(|| run_id.clone());
        let mut paused = resume_token
            .as_deref()
            .and_then(take_paused_task_scan)
            .unwrap_or_else(|| PausedTaskScan {
                files: session_file_paths(&home),
                next_index: 0,
                seen_task_ids: HashSet::new(),
                tasks: Vec::new(),
                discovered: 0,
                paused_at: Instant::now(),
            });
        let start_index = paused.next_index;
        let files = std::mem::take(&mut paused.files);
        let total = files.len();
        emit_scan_event(
            &app,
            serde_json::json!({
                "runId": run_id, "kind": "progress", "stage": "scanning", "scanned": start_index,
                "total": total, "discovered": paused.discovered, "resumed": resume_token.is_some()
            }),
        );
        let mut batch = Vec::new();
        let mut discovered = paused.discovered;
        let mut seen = paused.seen_task_ids;
        let mut preview_tasks = paused.tasks;
        for index in start_index..total {
            let path = &files[index];
            if job_is_cancelled(&run_id) {
                if !batch.is_empty() {
                    emit_scan_event(
                        &app,
                        serde_json::json!({
                            "runId": run_id, "kind": "batch", "stage": "scanning", "scanned": index,
                            "total": total, "discovered": discovered, "tasks": batch
                        }),
                    );
                }
                emit_scan_event(
                    &app,
                    serde_json::json!({
                        "runId": run_id, "kind": "cancelled", "stage": "scanning", "scanned": index,
                        "total": total, "discovered": discovered
                    }),
                );
                clear_background_job(&run_id);
                return;
            }
            if started.elapsed() > TASK_SCAN_TIMEOUT {
                if !batch.is_empty() {
                    emit_scan_event(
                        &app,
                        serde_json::json!({
                            "runId": run_id, "kind": "batch", "stage": "scanning", "scanned": index,
                            "total": total, "discovered": discovered, "tasks": batch
                        }),
                    );
                }
                store_paused_task_scan(
                    continuation_token.clone(),
                    PausedTaskScan {
                        files,
                        next_index: index,
                        seen_task_ids: seen,
                        tasks: preview_tasks,
                        discovered,
                        paused_at: Instant::now(),
                    },
                );
                emit_scan_event(
                    &app,
                    serde_json::json!({
                        "runId": run_id, "kind": "timed_out", "stage": "scanning", "scanned": index,
                        "total": total, "discovered": discovered, "resumeToken": continuation_token
                    }),
                );
                clear_background_job(&run_id);
                return;
            }
            if let Some(task) = scan_preview_task(&home, path) {
                if seen.insert(task.id.clone()) {
                    discovered += 1;
                    preview_tasks.push(task.clone());
                    batch.push(task);
                }
            }
            if batch.len() >= 24 || index + 1 == total {
                emit_scan_event(
                    &app,
                    serde_json::json!({
                        "runId": run_id, "kind": "batch", "stage": "scanning", "scanned": index + 1,
                        "total": total, "discovered": discovered, "tasks": batch
                    }),
                );
                batch = Vec::new();
            } else if (index + 1) % 12 == 0 {
                emit_scan_event(
                    &app,
                    serde_json::json!({
                        "runId": run_id, "kind": "progress", "stage": "scanning", "scanned": index + 1,
                        "total": total, "discovered": discovered
                    }),
                );
            }
        }

        // The second pass uses the in-memory session cache populated above. It
        // only enriches task rows with index/database/sidebar metadata and then
        // calculates the health summary.
        emit_scan_event(
            &app,
            serde_json::json!({
                "runId": run_id, "kind": "progress", "stage": "organizing", "scanned": total,
                "total": total, "discovered": discovered
            }),
        );
        if job_is_cancelled(&run_id) {
            emit_scan_event(
                &app,
                serde_json::json!({"runId": run_id, "kind": "cancelled", "stage": "organizing", "scanned": total, "total": total, "discovered": discovered}),
            );
        } else if let Ok((tasks, affected_title_ids)) =
            enrich_local_tasks_with_title_ids(&home, preview_tasks)
        {
            if let Ok(mut scans) = paused_task_scans().lock() {
                scans.remove(&continuation_token);
            }
            let health = task_health_report(&tasks, &affected_title_ids);
            emit_scan_event(
                &app,
                serde_json::json!({
                    "runId": run_id, "kind": "complete", "stage": "complete", "scanned": total,
                    "total": total, "discovered": tasks.len(), "tasks": tasks,
                    "health": health, "codexHome": home.to_string_lossy()
                }),
            );
        } else {
            emit_scan_event(
                &app,
                serde_json::json!({"runId": run_id, "kind": "error", "message": "读取本地任务元数据失败"}),
            );
        }
        clear_background_job(&run_id);
    });
    Ok(())
}

#[tauri::command]
fn cancel_background_job(job_id: String) -> bool {
    if job_id.trim().is_empty() {
        return false;
    }
    cancelled_background_jobs()
        .lock()
        .map(|mut jobs| jobs.insert(job_id))
        .unwrap_or(false)
}

#[tauri::command]
async fn load_task_library() -> Result<TaskLibrary, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let home = codex_home();
        let (tasks, affected_title_ids) = list_local_tasks_with_title_ids()?;
        Ok(TaskLibrary {
            health: task_health_report(&tasks, &affected_title_ids),
            tasks,
            codex_home: home.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn build_repair_plan(task_ids: Vec<String>) -> Result<RepairPlan, String> {
    tauri::async_runtime::spawn_blocking(move || build_repair_plan_for(&task_ids))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn apply_repair_plan(task_ids: Vec<String>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || apply_repair_plan_blocking(task_ids))
        .await
        .map_err(|error| error.to_string())?
}

fn apply_repair_plan_blocking(task_ids: Vec<String>) -> Result<serde_json::Value, String> {
    let plan = build_repair_plan_for(&task_ids)?;
    let executable_ids = plan
        .items
        .iter()
        .filter(|item| item.can_apply)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let registered_count = plan
        .items
        .iter()
        .filter(|item| item.can_apply && item.actions.iter().any(|action| action == "reregister"))
        .count();
    let title_repair_count = plan
        .items
        .iter()
        .filter(|item| item.can_apply && item.actions.iter().any(|action| action == "repair_title"))
        .count();

    if executable_ids.is_empty() {
        return Ok(serde_json::json!({
            "receipt": {
                "scanned": plan.items.len(),
                "registered": 0,
                "titlesRepaired": 0,
                "skipped": plan.manual_review_count,
                "backups": [],
                "codexHome": codex_home(),
                "message": "没有可安全执行的修复；请查看人工处理原因。"
            },
            "plan": plan
        }));
    }

    let home = codex_home();
    let _write_guard = acquire_local_write_operation(&home)?;
    let result = restore_local_tasks_blocking(executable_ids)?;
    let backups = result
        .get("backups")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let codex_home_value = result
        .get("codexHome")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(codex_home()));
    let mut response = serde_json::json!({
        "receipt": {
            "scanned": plan.items.len(),
            "registered": registered_count,
            "titlesRepaired": title_repair_count,
            "skipped": plan.manual_review_count,
            "backups": backups,
            "codexHome": codex_home_value,
            "message": "修复已完成。重新打开 Codex 后可检查任务是否重新出现。"
        },
        "plan": plan,
        "result": result
    });
    let operation_receipt = append_operation_receipt(&codex_home(), "repair", &response);
    if let Some(receipt) = response.get_mut("receipt").and_then(Value::as_object_mut) {
        match operation_receipt {
            Ok(receipt_path) => {
                receipt.insert("receiptPath".to_string(), Value::String(receipt_path));
            }
            Err(error) => {
                receipt.insert(
                    "receiptWarning".to_string(),
                    Value::String(format!("修复已完成，但无法写入维护回执：{error}")),
                );
            }
        }
    }
    Ok(response)
}

#[tauri::command]
fn list_tasks() -> Result<TaskList, String> {
    let home = codex_home();
    let tasks = list_local_tasks()?;
    Ok(TaskList {
        bad_title_count: 0,
        tasks,
        codex_home: home.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn export_tasks(task_ids: Vec<String>, destination: String) -> Result<serde_json::Value, String> {
    let requested: HashSet<_> = task_ids.into_iter().collect();
    let mut selected: Vec<_> = list_local_tasks()?
        .into_iter()
        .filter(|task| requested.contains(&task.id))
        .collect();
    if selected.is_empty() {
        return Err("请至少选择一个任务".to_string());
    }
    if let Some(state) = read_desktop_project_state(&codex_home()) {
        sort_tasks_by_project_order(&mut selected, &state.project_order);
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
fn restore_local_tasks(task_ids: Vec<String>) -> Result<serde_json::Value, String> {
    let home = codex_home();
    let _write_guard = acquire_local_write_operation(&home)?;
    restore_local_tasks_blocking(task_ids)
}

fn restore_local_tasks_blocking(task_ids: Vec<String>) -> Result<serde_json::Value, String> {
    if is_codex_desktop_running() {
        return Err("检测到 Codex/ChatGPT 桌面端正在运行。请先完全退出 Codex，再恢复本地会话，避免侧边栏状态被运行中的客户端覆盖。".to_string());
    }
    let requested: HashSet<_> = task_ids.into_iter().collect();
    let home = codex_home();
    let mut selected: Vec<_> = list_local_tasks()?
        .into_iter()
        .filter(|task| requested.contains(&task.id))
        .collect();
    if selected.is_empty() {
        return Err("请至少选择一个任务".to_string());
    }
    let model_settings = codex_model_settings(&home);
    for task in &mut selected {
        task.archived = false;
        if task.updated_at.is_empty() {
            task.updated_at = Utc::now().to_rfc3339();
        }
        if task.created_at.is_empty() {
            task.created_at = task.updated_at.clone();
        }
        if task.title.is_empty() {
            task.title = truncate(&task.first_user_message, 96);
        }
        normalize_task_title(task);
        if task.title.is_empty() {
            task.title = format!("未命名任务 {}", &task.id[..task.id.len().min(8)]);
        }
        // A restored task must resume with the API/model configuration that is
        // active on this computer, rather than a provider recorded by an older
        // API endpoint.
        normalize_task_to_codex_model(task, &model_settings);
    }
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    checkpoint_sqlite_database(&latest_state_database(&home))?;
    checkpoint_sqlite_database(&home.join("sqlite").join("codex-dev.db"))?;
    let mut transaction = ImportTransaction::new(&home);
    for path in codex_state_paths(&home) {
        transaction.backup(&path, &stamp)?;
    }
    for task in &mut selected {
        let contents = fs::read_to_string(&task.file_path).map_err(|error| error.to_string())?;
        let rewritten = rewrite_session_model_context(&contents, &model_settings);
        let archived_source = task.file_path.starts_with(home.join("archived_sessions"));
        if archived_source {
            let active_path = active_session_copy_path(&home, task);
            if let Some(parent) = active_path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            // Keep the archived source as a recoverable historical copy. The
            // active copy becomes authoritative on the next scan and prevents
            // a successfully re-registered task from being flagged again.
            transaction.write_file(&active_path, rewritten, &stamp)?;
            task.file_path = active_path;
        } else if rewritten != contents {
            transaction.write_file(&task.file_path, rewritten, &stamp)?;
        }
    }
    append_index(&home, &selected)?;
    register_threads(&home, &selected)?;
    register_catalog_threads(&home, &selected)?;
    register_desktop_project_state(&home, &selected)?;
    verify_registered_threads(&home, &selected)?;
    let backups = transaction.commit();
    Ok(
        serde_json::json!({"restored": selected.iter().map(|task| serde_json::json!({"id": task.id, "title": task.title, "cwd": task.cwd, "rolloutPath": task.file_path})).collect::<Vec<_>>(), "backups": backups, "codexHome": home}),
    )
}

#[tauri::command]
async fn bind_local_tasks(
    task_ids: Vec<String>,
    target_cwd: String,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || bind_local_tasks_blocking(task_ids, target_cwd))
        .await
        .map_err(|error| error.to_string())?
}

fn bind_local_tasks_blocking(
    task_ids: Vec<String>,
    target_cwd: String,
) -> Result<serde_json::Value, String> {
    if is_codex_desktop_running() {
        return Err("检测到 Codex/ChatGPT 桌面端正在运行。请先完全退出 Codex，再绑定本机项目，避免侧边栏状态被运行中的客户端覆盖。".to_string());
    }
    let target_cwd = target_cwd.trim();
    if target_cwd.is_empty() {
        return Err("请选择本机项目目录".to_string());
    }
    let target_cwd = fs::canonicalize(target_cwd)
        .map_err(|error| format!("无法访问本机项目目录：{error}"))?;
    if !target_cwd.is_dir() {
        return Err("目标路径不是文件夹".to_string());
    }
    let requested: HashSet<_> = task_ids.into_iter().collect();
    if requested.is_empty() {
        return Err("请至少选择一个任务".to_string());
    }

    let home = codex_home();
    let _write_guard = acquire_local_write_operation(&home)?;
    let mut selected: Vec<_> = list_local_tasks()?
        .into_iter()
        .filter(|task| requested.contains(&task.id))
        .collect();
    if selected.is_empty() {
        return Err("本机没有找到可对应的会话文件".to_string());
    }
    let target_cwd = target_cwd.to_string_lossy().to_string();
    let mut rewritten_sessions = Vec::with_capacity(selected.len());
    for task in &selected {
        let contents = fs::read_to_string(&task.file_path).map_err(|error| error.to_string())?;
        let rewritten = if task.cwd.trim().is_empty() {
            rewrite_session_meta_cwd(&contents, &target_cwd)
        } else {
            rewrite_session_meta_cwd(
                &rewrite_session_cwd(&contents, &task.cwd, &target_cwd),
                &target_cwd,
            )
        };
        rewritten_sessions.push((task.file_path.clone(), rewritten));
    }
    for task in &mut selected {
        task.cwd = target_cwd.clone();
        task.archived = false;
        (
            task.project_key,
            task.project_name,
            task.project_path,
            task.project_exists,
        ) = infer_project(task);
    }

    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    checkpoint_sqlite_database(&latest_state_database(&home))?;
    checkpoint_sqlite_database(&home.join("sqlite").join("codex-dev.db"))?;
    let mut transaction = ImportTransaction::new(&home);
    for path in codex_state_paths(&home) {
        transaction.backup(&path, &stamp)?;
    }
    for (path, rewritten) in rewritten_sessions {
        transaction.write_file(&path, rewritten, &stamp)?;
    }
    append_index(&home, &selected)?;
    register_threads(&home, &selected)?;
    register_catalog_threads(&home, &selected)?;
    register_desktop_project_state(&home, &selected)?;
    verify_registered_threads(&home, &selected)?;
    let backups = transaction.commit();
    let mut response = serde_json::json!({
        "bound": selected.iter().map(|task| serde_json::json!({
            "id": task.id,
            "title": task.title,
            "cwd": task.cwd
        })).collect::<Vec<_>>(),
        "targetCwd": target_cwd,
        "backups": backups,
        "codexHome": home,
        "message": "已绑定到本机项目。重新打开 Codex 后检查侧边栏。"
    });
    match append_operation_receipt(&home, "bind-project", &response) {
        Ok(path) => {
            if let Some(object) = response.as_object_mut() {
                object.insert("receiptPath".to_string(), Value::String(path));
            }
        }
        Err(error) => {
            if let Some(object) = response.as_object_mut() {
                object.insert("receiptWarning".to_string(), Value::String(error));
            }
        }
    }
    Ok(response)
}

#[tauri::command]
fn inspect_archive(archive_path: String) -> Result<ArchiveInspection, String> {
    let manifest = archive_manifest(Path::new(&archive_path))?;
    let model_settings = codex_model_settings(&codex_home());
    let file = File::open(&archive_path).map_err(|_| "找不到所选压缩包".to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|_| "这不是有效的 ZIP 压缩包".to_string())?;
    let existing: HashMap<_, _> = list_local_tasks()?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    Ok(ArchiveInspection {
        canceled: false,
        path: archive_path,
        created_at: manifest.created_at,
        tasks: manifest
            .tasks
            .into_iter()
            .map(|item| {
                let mut task = item.task;
                let mut contents = String::new();
                if let Ok(mut file) = zip.by_name(&item.session_file) {
                    if file.read_to_string(&mut contents).is_ok() {
                        hydrate_task_from_session_content(&mut task, &contents);
                    }
                }
                normalize_task_title(&mut task);
                if task.title.is_empty() {
                    task.title = format!("未命名任务 {}", &task.id[..task.id.len().min(8)]);
                }
                if task.model_provider.trim() == "custom" {
                    normalize_task_to_codex_model(&mut task, &model_settings);
                }
                let merge_preview = existing
                    .get(&task.id)
                    .and_then(|local_task| fs::read_to_string(&local_task.file_path).ok())
                    .map(|local_contents| {
                        safe_merge_session_jsonl(&contents, &local_contents).preview
                    });
                InspectedTask {
                    conflict: existing.contains_key(&task.id),
                    merge_preview,
                    task,
                }
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
    let _write_guard = acquire_local_write_operation(&home)?;
    let model_settings = codex_model_settings(&home);
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
    let merge_task_ids = options
        .as_ref()
        .and_then(|value| value.merge_task_ids.clone())
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let target_cwd = options
        .as_ref()
        .and_then(|value| value.target_cwd.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    checkpoint_sqlite_database(&latest_state_database(&home))?;
    checkpoint_sqlite_database(&home.join("sqlite").join("codex-dev.db"))?;
    let mut transaction = ImportTransaction::new(&home);
    let mut imported = Vec::new();
    let mut restored = Vec::new();
    let mut merged = Vec::new();
    let mut skipped = Vec::new();
    for entry in manifest.tasks {
        let mut archive_task = entry.task;
        let mut content = String::new();
        zip.by_name(&entry.session_file)
            .map_err(|_| "压缩包缺少会话文件".to_string())?
            .read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        hydrate_task_from_session_content(&mut archive_task, &content);
        normalize_task_title(&mut archive_task);
        // Archives are history, not portable API configuration.  Always adapt
        // the resumable context to the receiving Codex installation.
        let normalize_to_codex = true;
        if normalize_to_codex {
            normalize_task_to_codex_model(&mut archive_task, &model_settings);
        }

        if let Some(existing_task) = existing_tasks.get(&archive_task.id) {
            if restore_existing {
                let mut task = existing_task.clone();
                let merge_result = if merge_task_ids.contains(&archive_task.id) {
                    let local_contents = match fs::read_to_string(&task.file_path) {
                        Ok(contents) => contents,
                        Err(_) => {
                            skipped.push(serde_json::json!({"id": archive_task.id, "title": archive_task.title, "reason": "merge_local_session_unreadable"}));
                            continue;
                        }
                    };
                    let result = safe_merge_session_jsonl(&content, &local_contents);
                    if !result.preview.can_merge {
                        skipped.push(serde_json::json!({"id": archive_task.id, "title": archive_task.title, "reason": "merge_requires_manual_review", "detail": result.preview.reason}));
                        continue;
                    }
                    Some(result)
                } else {
                    None
                };
                let normalize_existing =
                    normalize_to_codex || task.model_provider.trim() == "custom";
                let original_cwd = task.cwd.clone();
                if is_bad_title(&task.title) && !is_bad_title(&archive_task.title) {
                    task.title = archive_task.title.clone();
                }
                if meaningful_user_text(&task.first_user_message).is_none()
                    && meaningful_user_text(&archive_task.first_user_message).is_some()
                {
                    task.first_user_message = archive_task.first_user_message.clone();
                }
                if meaningful_user_text(&task.preview).is_none()
                    && meaningful_user_text(&archive_task.preview).is_some()
                {
                    task.preview = archive_task.preview.clone();
                }
                replace_if_present(&mut task.source, &archive_task.source);
                replace_if_present(&mut task.model_provider, &archive_task.model_provider);
                replace_if_present(&mut task.model, &archive_task.model);
                replace_if_present(&mut task.reasoning_effort, &archive_task.reasoning_effort);
                replace_if_present(&mut task.sandbox_policy, &archive_task.sandbox_policy);
                replace_if_present(&mut task.approval_mode, &archive_task.approval_mode);
                replace_if_present(&mut task.cli_version, &archive_task.cli_version);
                replace_if_present(&mut task.thread_source, &archive_task.thread_source);
                replace_if_present(&mut task.forked_from_id, &archive_task.forked_from_id);
                replace_if_present(&mut task.agent_path, &archive_task.agent_path);
                replace_if_present(&mut task.agent_nickname, &archive_task.agent_nickname);
                replace_if_present(&mut task.agent_role, &archive_task.agent_role);
                replace_if_present(&mut task.memory_mode, &archive_task.memory_mode);
                replace_if_present(&mut task.history_mode, &archive_task.history_mode);
                if normalize_existing {
                    normalize_task_to_codex_model(&mut task, &model_settings);
                }
                normalize_task_title(&mut task);
                let resolved_cwd =
                    resolved_import_cwd(&task.cwd, target_cwd.as_deref(), adapt_paths);
                let cwd_changed = resolved_cwd != task.cwd;
                if cwd_changed {
                    task.cwd = resolved_cwd;
                }
                if cwd_changed || normalize_existing || merge_result.is_some() {
                    let mut contents = if let Some(result) = &merge_result {
                        result.contents.clone()
                    } else if let Ok(contents) = fs::read_to_string(&task.file_path) {
                        contents
                    } else {
                        skipped.push(serde_json::json!({"id": archive_task.id, "title": archive_task.title, "reason": "local_session_unreadable"}));
                        continue;
                    };
                    if cwd_changed {
                        contents = rewrite_session_cwd(&contents, &original_cwd, &task.cwd);
                        contents = rewrite_session_meta_cwd(&contents, &task.cwd);
                    }
                    if normalize_existing {
                        contents = rewrite_session_model_context(&contents, &model_settings);
                    }
                    transaction.write_file(&task.file_path, contents, &stamp)?;
                }
                task.archived = false;
                (
                    task.project_key,
                    task.project_name,
                    task.project_path,
                    task.project_exists,
                ) = infer_project(&task);
                if let Some(result) = &merge_result {
                    merged.push(serde_json::json!({"id": task.id, "title": task.title, "appendedRecords": result.preview.append_record_count}));
                }
                restored.push(task);
            } else {
                skipped.push(serde_json::json!({"id": archive_task.id, "title": archive_task.title, "reason": "already_exists"}));
            }
            continue;
        }
        let mut task = archive_task;
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
        let local_cwd = resolved_import_cwd(&task.cwd, target_cwd.as_deref(), adapt_paths);
        let mut session_content = rewrite_session_cwd(&content, &task.cwd, &local_cwd);
        if local_cwd != task.cwd {
            session_content = rewrite_session_meta_cwd(&session_content, &local_cwd);
        }
        if normalize_to_codex {
            session_content = rewrite_session_model_context(&session_content, &model_settings);
            normalize_task_to_codex_model(&mut task, &model_settings);
        }
        transaction.write_file(&rollout_path, session_content, &stamp)?;
        if let Some(browser_file) = entry.browser_file {
            let mut contents = Vec::new();
            zip.by_name(&browser_file)
                .map_err(|_| "压缩包缺少浏览器配置".to_string())?
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            let browser_path = home
                .join("browser")
                .join("sessions")
                .join(format!("{}.toml", task.id));
            transaction.write_file(&browser_path, contents, &stamp)?;
        }
        task.cwd = local_cwd;
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
        normalize_task_title(&mut task);
        if task.title.is_empty() {
            task.title = format!("未命名任务 {}", &task.id[..task.id.len().min(8)]);
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
        let backups = transaction.commit();
        return Ok(
            serde_json::json!({"imported": [], "restored": [], "merged": [], "skipped": skipped, "backups": backups, "codexHome": home}),
        );
    }
    for path in codex_state_paths(&home) {
        transaction.backup(&path, &stamp)?;
    }
    let registered = imported
        .iter()
        .chain(restored.iter())
        .cloned()
        .collect::<Vec<_>>();
    append_index(&home, &registered)?;
    register_threads(&home, &registered)?;
    register_catalog_threads(&home, &registered)?;
    register_desktop_project_state(&home, &registered)?;
    verify_registered_threads(&home, &registered)?;
    let backups = transaction.commit();
    let mut response = serde_json::json!({"imported": imported.iter().map(|task| serde_json::json!({"id": task.id, "title": task.title, "cwd": task.cwd, "rolloutPath": task.file_path})).collect::<Vec<_>>(), "restored": restored.iter().map(|task| serde_json::json!({"id": task.id, "title": task.title, "cwd": task.cwd, "rolloutPath": task.file_path})).collect::<Vec<_>>(), "merged": merged, "skipped": skipped, "backups": backups, "codexHome": home});
    let operation_receipt = append_operation_receipt(&home, "import", &response);
    if let Some(object) = response.as_object_mut() {
        match operation_receipt {
            Ok(receipt_path) => {
                object.insert("receiptPath".to_string(), Value::String(receipt_path));
            }
            Err(error) => {
                object.insert(
                    "receiptWarning".to_string(),
                    Value::String(format!("导入已完成，但无法写入维护回执：{error}")),
                );
            }
        }
    }
    Ok(response)
}

#[tauri::command]
fn get_environment() -> serde_json::Value {
    let codex_processes = codex_desktop_processes();
    let home = codex_home();
    let model_settings = cached_environment_model_settings(&home);
    serde_json::json!({"codexHome": home, "platform": env::consts::OS, "version": env!("CARGO_PKG_VERSION"), "codexRunning": !codex_processes.is_empty(), "codexProcesses": codex_processes, "activeModelProvider": model_settings.provider, "activeModel": model_settings.model, "activeReasoningEffort": model_settings.reasoning_effort})
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
            load_task_library,
            start_task_scan,
            cancel_background_job,
            get_task_health,
            build_repair_plan,
            apply_repair_plan,
            validate_task_library,
            list_operation_receipts,
            get_operation_receipts_directory,
            list_local_snapshots,
            delete_local_snapshots,
            export_tasks,
            restore_local_tasks,
            bind_local_tasks,
            inspect_archive,
            import_archive,
            get_environment
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Session Transfer");
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_local_write_operation, append_index, archive_manifest, codex_model_settings,
        codex_state_paths, history_first_messages, infer_project, is_bad_title,
        is_codex_context_text, is_codex_desktop_process, is_safe_task_id, latest_state_database,
        local_snapshots, meaningful_user_text, prune_paused_task_scans, register_catalog_threads,
        register_desktop_project_state, register_threads, repository_name, rewrite_session_cwd,
        rewrite_session_model_context, safe_merge_session_jsonl, session_details,
        simple_toml_string, sort_tasks_by_project_order, validated_local_snapshot_path,
        verify_registered_threads, ArchiveTask,
        CodexModelSettings, ImportTransaction, Manifest, PausedTaskScan, Task, ARCHIVE_SCHEMA,
        LOCAL_SNAPSHOT_MARKER, LOCAL_WRITE_LOCK_FILE, MAX_PAUSED_TASK_SCANS,
        PAUSED_TASK_SCAN_TTL,
    };
    use rusqlite::Connection;
    use serde_json::Value;
    use std::{
        collections::{HashMap, HashSet},
        env, fs,
        io::Write,
        path::PathBuf,
        time::{Duration, Instant},
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn safe_merge_session_jsonl_appends_only_newer_local_records() {
        let archive = concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-07-01T10:00:00Z\",\"payload\":{\"id\":\"merge-task\"}}\n",
            "{\"type\":\"event_msg\",\"timestamp\":\"2026-07-01T10:01:00Z\",\"payload\":{\"type\":\"user_message\",\"message\":\"历史消息\"}}\n"
        );
        let local = concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-07-01T10:00:00Z\",\"payload\":{\"id\":\"merge-task\"}}\n",
            "{\"type\":\"event_msg\",\"timestamp\":\"2026-07-01T10:01:00Z\",\"payload\":{\"type\":\"user_message\",\"message\":\"历史消息\"}}\n",
            "{\"type\":\"event_msg\",\"timestamp\":\"2026-07-01T10:02:00Z\",\"payload\":{\"type\":\"user_message\",\"message\":\"本机续聊\"}}\n"
        );

        let merged = safe_merge_session_jsonl(archive, local);

        assert!(merged.preview.can_merge);
        assert_eq!(merged.preview.append_record_count, 1);
        assert_eq!(merged.contents.lines().count(), 3);
        assert!(merged.contents.contains("本机续聊"));
    }

    #[test]
    fn safe_merge_session_jsonl_rejects_missing_comparable_timestamps() {
        let archive = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"merge-task\"}}\n";
        let local = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"本机续聊\"}}\n";

        let merged = safe_merge_session_jsonl(archive, local);

        assert!(!merged.preview.can_merge);
        assert_eq!(merged.preview.append_record_count, 0);
        assert!(merged.preview.reason.contains("时间戳"));
    }

    #[test]
    fn detects_codex_and_chatgpt_from_any_macos_application_path() {
        assert!(is_codex_desktop_process(
            "123 /Users/test/Applications/Codex.app/Contents/MacOS/Codex"
        ));
        assert!(is_codex_desktop_process(
            "456 /private/var/folders/tmp/AppTranslocation/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
        assert!(!is_codex_desktop_process(
            "789 /Applications/Terminal.app/Contents/MacOS/Terminal"
        ));
    }

    #[test]
    fn local_write_operation_rejects_parallel_writes() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-write-lock-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&home).ok();
        fs::create_dir_all(&home).unwrap();
        let guard =
            acquire_local_write_operation(&home).expect("first write operation acquires the lock");
        assert!(home.join(LOCAL_WRITE_LOCK_FILE).exists());
        assert!(acquire_local_write_operation(&home).is_err());
        drop(guard);
        assert!(!home.join(LOCAL_WRITE_LOCK_FILE).exists());
        fs::remove_dir_all(home).ok();
    }

    #[test]
    fn local_snapshots_only_lists_backup_files() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-snapshots-{}",
            std::process::id()
        ));
        fs::create_dir_all(home.join("sqlite")).unwrap();
        fs::write(
            home.join(format!("state_5.sqlite{LOCAL_SNAPSHOT_MARKER}test")),
            "backup",
        )
        .unwrap();
        fs::write(
            home.join("sqlite")
                .join(format!("codex-dev.db{LOCAL_SNAPSHOT_MARKER}test")),
            "backup",
        )
        .unwrap();
        fs::write(home.join("state_5.sqlite"), "live").unwrap();

        let snapshots = local_snapshots(&home).unwrap();
        fs::remove_dir_all(&home).ok();

        assert_eq!(snapshots.len(), 2);
        assert!(snapshots
            .iter()
            .all(|snapshot| snapshot.name.contains(LOCAL_SNAPSHOT_MARKER)));
    }

    #[test]
    fn snapshot_deletion_rejects_files_outside_codex_home() {
        let root = env::temp_dir().join(format!(
            "codex-session-transfer-snapshot-scope-{}",
            std::process::id()
        ));
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let outside = root.join(format!("outside{LOCAL_SNAPSHOT_MARKER}test"));
        fs::write(&outside, "backup").unwrap();

        let result = validated_local_snapshot_path(&home, &outside.to_string_lossy());
        fs::remove_dir_all(&root).ok();

        assert!(result.is_err());
    }

    fn paused_scan(paused_at: Instant) -> PausedTaskScan {
        PausedTaskScan {
            files: Vec::new(),
            next_index: 0,
            seen_task_ids: HashSet::new(),
            tasks: Vec::new(),
            discovered: 0,
            paused_at,
        }
    }

    #[test]
    fn paused_scan_cache_discards_expired_and_oldest_entries() {
        let now = Instant::now();
        let mut scans = HashMap::new();
        scans.insert(
            "expired".to_string(),
            paused_scan(now - PAUSED_TASK_SCAN_TTL),
        );
        for index in 0..=MAX_PAUSED_TASK_SCANS {
            scans.insert(
                format!("scan-{index}"),
                paused_scan(now - Duration::from_secs((MAX_PAUSED_TASK_SCANS - index) as u64)),
            );
        }

        prune_paused_task_scans(&mut scans);

        assert_eq!(scans.len(), MAX_PAUSED_TASK_SCANS);
        assert!(!scans.contains_key("expired"));
        assert!(!scans.contains_key("scan-0"));
        assert!(scans.contains_key(&format!("scan-{MAX_PAUSED_TASK_SCANS}")));
    }

    #[test]
    fn latest_state_database_prefers_the_highest_numeric_version() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-latest-state-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("state_5.sqlite"), "").unwrap();
        fs::write(home.join("state_12.sqlite"), "").unwrap();

        assert_eq!(latest_state_database(&home), home.join("state_12.sqlite"));
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn history_messages_keep_the_first_readable_user_message_per_session() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-history-title-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("history.jsonl"),
            concat!(
                "{\"session_id\":\"history-task\",\"text\":\"第一个问题\"}\n",
                "{\"session_id\":\"history-task\",\"text\":\"第二个问题\"}\n"
            ),
        )
        .unwrap();

        assert_eq!(
            history_first_messages(&home)
                .get("history-task")
                .map(String::as_str),
            Some("第一个问题")
        );
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn rewrite_session_cwd_only_updates_structured_path_fields() {
        let source = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/old\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"cwd\":\"/old/work\",\"writable_roots\":[\"/old\",\"/other\"]}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Use /old/file in this command\"}}\n"
        );
        let rewritten = rewrite_session_cwd(source, "/old", "/new");
        let lines = rewritten
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(lines[0]["payload"]["cwd"].as_str(), Some("/new"));
        assert_eq!(lines[1]["payload"]["cwd"].as_str(), Some("/new/work"));
        assert_eq!(
            lines[1]["payload"]["writable_roots"][0].as_str(),
            Some("/new")
        );
        assert_eq!(
            lines[2]["payload"]["message"].as_str(),
            Some("Use /old/file in this command")
        );
    }

    #[test]
    fn rewrite_session_cwd_handles_a_changed_windows_username() {
        let source = concat!(
            r#"{"type":"session_meta","payload":{"cwd":"C:\\Users\\HUAWEI\\Projects\\demo"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"cwd":"C:\\Users\\HUAWEI\\Projects\\demo","writable_roots":["C:\\Users\\HUAWEI\\Projects\\demo"]}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"C:\\Users\\HUAWEI\\Projects\\demo should remain quoted"}}"#,
            "\n"
        );
        let rewritten = rewrite_session_cwd(
            source,
            r#"C:\Users\HUAWEI\Projects\demo"#,
            r#"C:\Users\Legion\Projects\demo"#,
        );
        let lines = rewritten
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            lines[0]["payload"]["cwd"].as_str(),
            Some(r#"C:\Users\Legion\Projects\demo"#)
        );
        assert_eq!(
            lines[1]["payload"]["writable_roots"][0].as_str(),
            Some(r#"C:\Users\Legion\Projects\demo"#)
        );
        assert_eq!(
            lines[2]["payload"]["message"].as_str(),
            Some(r#"C:\Users\HUAWEI\Projects\demo should remain quoted"#)
        );
    }

    #[test]
    fn context_title_filter_keeps_normal_messages_that_mention_cwd() {
        assert!(!is_codex_context_text("请解释 <cwd> 在命令行里代表什么"));
        assert!(is_codex_context_text(
            "<environment_context> <cwd>/tmp/project</cwd> </environment_context>"
        ));
    }

    #[test]
    fn title_filter_removes_embedded_image_and_tool_transcript() {
        let raw = "优化一下界面排版 <image name=[Image #1] path=\"C:\\Temp\\layout.png\"> assistant to=functions.exec tool exec call: ignored";
        assert!(is_bad_title(raw));
        assert_eq!(
            meaningful_user_text(raw).as_deref(),
            Some("优化一下界面排版")
        );
    }

    #[test]
    fn archive_manifest_rejects_missing_declared_browser_file() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-missing-browser-{}.zip",
            std::process::id()
        ));
        let file = fs::File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let mut task = task_with("测试任务", "/tmp/project", "");
        task.id = "missing-browser-task".to_string();
        let manifest = Manifest {
            schema: ARCHIVE_SCHEMA.to_string(),
            created_at: "2026-07-22T12:00:00Z".to_string(),
            source_platform: "macos".to_string(),
            tasks: vec![ArchiveTask {
                task,
                session_file: "tasks/missing-browser-task/session.jsonl".to_string(),
                browser_file: Some("tasks/missing-browser-task/browser.toml".to_string()),
            }],
        };
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        zip.start_file(
            "tasks/missing-browser-task/session.jsonl",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"{}\n").unwrap();
        zip.finish().unwrap();

        let result = archive_manifest(&path);
        fs::remove_file(path).ok();

        assert!(result.is_err());
    }

    #[test]
    fn archive_manifest_rejects_session_id_mismatch() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-id-mismatch-{}.zip",
            std::process::id()
        ));
        let file = fs::File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let mut task = task_with("测试任务", "/tmp/project", "");
        task.id = "manifest-task-id".to_string();
        let manifest = Manifest {
            schema: ARCHIVE_SCHEMA.to_string(),
            created_at: "2026-07-22T12:00:00Z".to_string(),
            source_platform: "macos".to_string(),
            tasks: vec![ArchiveTask {
                task,
                session_file: "tasks/manifest-task-id/session.jsonl".to_string(),
                browser_file: None,
            }],
        };
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        zip.start_file(
            "tasks/manifest-task-id/session.jsonl",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"type":"session_meta","payload":{"id":"session-task-id"}}"#)
            .unwrap();
        zip.finish().unwrap();

        let result = archive_manifest(&path);
        fs::remove_file(path).ok();

        assert!(result.is_err());
    }

    #[test]
    fn safe_task_ids_reject_path_separators() {
        assert!(is_safe_task_id("019f5acb-2182-70e3-832f-36994b7d12b"));
        assert!(!is_safe_task_id("task/../../outside"));
        assert!(!is_safe_task_id(r"task\\outside"));
    }

    #[test]
    fn import_transaction_restores_changed_files_and_removes_new_files() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-transaction-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        let existing = home.join("session_index.jsonl");
        let created = home.join("new-session.jsonl");
        fs::write(&existing, "before").unwrap();
        {
            let mut transaction = ImportTransaction::new(&home);
            transaction.write_file(&existing, "after", "test").unwrap();
            transaction.write_file(&created, "new", "test").unwrap();
        }

        assert_eq!(fs::read_to_string(&existing).unwrap(), "before");
        assert!(!created.exists());
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn session_fallback_keeps_provider_model_and_effort_together() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-current-settings-{}",
            std::process::id()
        ));
        let session_dir = home.join("sessions").join("2026").join("07").join("24");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            home.join("config.toml"),
            "model = \"gpt-stale\"\nmodel_reasoning_effort = \"low\"\n",
        )
        .unwrap();
        fs::write(
            session_dir.join("rollout-2026-07-24T12-00-00-settings.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"custom\",\"model\":\"glm-old\",\"reasoning_effort\":\"low\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"model_provider_id\":\"openai\",\"model\":\"gpt-current\",\"reasoning_effort\":\"high\"}}}\n"
            ),
        )
        .unwrap();

        let settings = codex_model_settings(&home);
        fs::remove_dir_all(&home).ok();

        assert_eq!(settings.provider, "openai");
        assert_eq!(settings.model, "gpt-current");
        assert_eq!(settings.reasoning_effort, "high");
    }

    #[test]
    fn session_fallback_skips_incomplete_newer_session() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-incomplete-settings-{}",
            std::process::id()
        ));
        let session_dir = home.join("sessions").join("2026").join("07").join("24");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout-2026-07-24T12-00-00-complete.jsonl"),
            r#"{"type":"session_meta","payload":{"model_provider":"custom","model":"glm-5.2","reasoning_effort":"high"}}"#,
        )
        .unwrap();
        fs::write(
            session_dir.join("rollout-2026-07-24T13-00-00-incomplete.jsonl"),
            r#"{"type":"session_meta","payload":{"model_provider":"openai"}}"#,
        )
        .unwrap();

        let settings = codex_model_settings(&home);
        fs::remove_dir_all(&home).ok();

        assert_eq!(settings.provider, "custom");
        assert_eq!(settings.model, "glm-5.2");
        assert_eq!(settings.reasoning_effort, "high");
    }

    #[test]
    fn session_fallback_ignores_partial_provider_switch() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-partial-provider-switch-{}",
            std::process::id()
        ));
        let session_dir = home.join("sessions").join("2026").join("07").join("24");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout-2026-07-24T12-00-00-partial.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"custom\",\"model\":\"glm-5.2\",\"reasoning_effort\":\"high\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"model_provider_id\":\"openai\"}}}\n"
            ),
        )
        .unwrap();

        let settings = codex_model_settings(&home);
        fs::remove_dir_all(&home).ok();

        assert_eq!(settings.provider, "custom");
        assert_eq!(settings.model, "glm-5.2");
        assert_eq!(settings.reasoning_effort, "high");
    }

    #[test]
    fn toml_reader_uses_root_key_and_ignores_comment() {
        let config = concat!(
            "model_provider = \"openai\" # active provider\n",
            "[profiles.custom]\n",
            "model_provider = \"custom\"\n"
        );
        assert_eq!(
            simple_toml_string(config, "model_provider"),
            Some("openai".to_string())
        );
    }

    #[test]
    fn transaction_restores_wal_sidecar_files() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-wal-rollback-{}",
            std::process::id()
        ));
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let state_wal = home.join("state_5.sqlite-wal");
        fs::write(&state_wal, "before").unwrap();
        {
            let mut transaction = ImportTransaction::new(&home);
            for path in codex_state_paths(&home) {
                transaction.backup(&path, "test").unwrap();
            }
            fs::write(&state_wal, "after").unwrap();
        }

        assert_eq!(fs::read_to_string(&state_wal).unwrap(), "before");
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn registered_thread_verification_requires_a_codex_database() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-missing-database-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        let result = verify_registered_threads(&home, &[task_with("task", "/tmp/project", "")]);
        fs::remove_dir_all(&home).ok();

        assert!(result.is_err());
    }

    #[test]
    fn registered_thread_verification_allows_state_only_when_catalog_schema_is_missing() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-state-only-verification-{}",
            std::process::id()
        ));
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let state = Connection::open(home.join("state_5.sqlite")).unwrap();
        state
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT, title TEXT, cwd TEXT, first_user_message TEXT, archived INTEGER,
                    source TEXT, model_provider TEXT, model TEXT, reasoning_effort TEXT,
                    sandbox_policy TEXT, approval_mode TEXT, cli_version TEXT, thread_source TEXT,
                    agent_path TEXT, agent_nickname TEXT, agent_role TEXT, memory_mode TEXT,
                    history_mode TEXT
                );
                INSERT INTO threads (id, archived) VALUES ('state-only-task', 0);
                "#,
            )
            .unwrap();
        drop(state);
        Connection::open(home.join("sqlite").join("codex-dev.db")).unwrap();
        let mut task = task_with("task", "/tmp/project", "");
        task.id = "state-only-task".to_string();

        let result = verify_registered_threads(&home, &[task]);
        fs::remove_dir_all(&home).ok();

        assert!(result.is_ok());
    }

    #[test]
    fn empty_cwd_registers_an_unbound_project_without_home_directory() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-unbound-project-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(".codex-global-state.json"), "{}").unwrap();
        let mut task = task_with("task", "", "");
        task.id = "unbound-task".to_string();

        register_desktop_project_state(&home, &[task]).unwrap();
        let value: Value = serde_json::from_str(
            &fs::read_to_string(home.join(".codex-global-state.json")).unwrap(),
        )
        .unwrap();
        fs::remove_dir_all(&home).ok();

        let state = &value["electron-persisted-atom-state"];
        assert_eq!(
            state["thread-project-assignments"]["unbound-task"]["cwd"],
            ""
        );
        assert_eq!(
            state["local-projects"]["unbound-unbound-task"]["rootPaths"],
            serde_json::json!([])
        );
    }

    #[test]
    fn append_index_preserves_existing_metadata_fields() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-index-metadata-{}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("session_index.jsonl"),
            r#"{"id":"index-task","thread_name":"before","updated_at":"old","future_field":"keep"}"#,
        )
        .unwrap();
        let mut task = task_with("after", "/tmp/project", "");
        task.id = "index-task".to_string();
        task.updated_at = "new".to_string();

        append_index(&home, &[task]).unwrap();
        let value: Value =
            serde_json::from_str(&fs::read_to_string(home.join("session_index.jsonl")).unwrap())
                .unwrap();
        fs::remove_dir_all(&home).ok();

        assert_eq!(value["thread_name"], "after");
        assert_eq!(value["updated_at"], "new");
        assert_eq!(value["future_field"], "keep");
    }

    #[test]
    fn session_details_preserves_custom_model_metadata() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-model-metadata-{}.jsonl",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"custom-model-task","timestamp":"2026-07-19T12:00:00Z","cwd":"/tmp/project","source":"vscode","thread_source":"user","cli_version":"0.145.0-alpha.18","model_provider":"custom","model":"glm-5.2","reasoning_effort":"high","memory_mode":"enabled","history_mode":"legacy"}}"#,
        )
        .unwrap();
        let task = session_details(&path).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(task.model_provider, "custom");
        assert_eq!(task.model, "glm-5.2");
        assert_eq!(task.reasoning_effort, "high");
        assert_eq!(task.cli_version, "0.145.0-alpha.18");
        assert_eq!(task.thread_source, "user");
    }

    #[test]
    fn session_details_reads_forked_from_id() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-forked-task-{}.jsonl",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"child-task","forked_from_id":"parent-task","timestamp":"2026-07-31T01:46:41Z","cwd":"/tmp/project"}}"#,
        )
        .unwrap();
        let task = session_details(&path).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(task.forked_from_id, "parent-task");
    }

    #[test]
    fn session_details_reads_model_from_turn_context() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-turn-context-model-{}.jsonl",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"custom-model-task","timestamp":"2026-07-19T12:00:00Z","cwd":"/tmp/project","source":"vscode","thread_source":"user","cli_version":"0.145.0-alpha.18","model_provider":"custom"}}
{"type":"turn_context","payload":{"model":"glm-5.2","effort":"high","approval_policy":"never","sandbox_policy":{"type":"danger-full-access"},"collaboration_mode":{"settings":{"reasoning_effort":"high"}}}}"#,
        )
        .unwrap();
        let task = session_details(&path).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(task.model_provider, "custom");
        assert_eq!(task.model, "glm-5.2");
        assert_eq!(task.reasoning_effort, "high");
        assert_eq!(task.sandbox_policy, r#"{"type":"danger-full-access"}"#);
        assert_eq!(task.approval_mode, "never");
    }

    #[test]
    fn session_details_uses_last_record_timestamp_for_updated_at() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-last-activity-{}.jsonl",
            std::process::id()
        ));
        // Rollout records carry their own timestamps; the final record's
        // timestamp must win over the freshly-written file's modification time.
        fs::write(
            &path,
            r#"{"type":"session_meta","timestamp":"2026-07-20T09:00:00Z","payload":{"id":"last-activity-task","timestamp":"2026-07-20T09:00:00Z","cwd":"/tmp/project","source":"vscode"}}
{"type":"event_msg","timestamp":"2026-07-20T09:30:00Z","payload":{"type":"user_message","message":"最后一次对话"}}"#,
        )
        .unwrap();
        let task = session_details(&path).unwrap();
        fs::remove_file(&path).ok();

        let expected = chrono::DateTime::parse_from_rfc3339("2026-07-20T09:30:00Z")
            .unwrap()
            .timestamp();
        let actual = chrono::DateTime::parse_from_rfc3339(&task.updated_at)
            .unwrap()
            .timestamp();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rewrite_session_model_context_converts_custom_to_codex_model() {
        let settings = CodexModelSettings {
            provider: "openai".to_string(),
            model: "gpt-5.5".to_string(),
            reasoning_effort: "high".to_string(),
        };
        let source = r#"{"type":"session_meta","payload":{"id":"custom-model-task","model_provider":"custom"}}
{"type":"turn_context","payload":{"model":"glm-5.2","effort":"high","collaboration_mode":{"settings":{"model":"glm-5.2","reasoning_effort":"high"}}}}
{"payload":{"type":"thread_settings_applied","thread_settings":{"model_provider_id":"custom","model":"glm-5.2","reasoning_effort":"high"}}}"#;
        let rewritten = rewrite_session_model_context(source, &settings);
        let lines = rewritten
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            lines[0]["payload"]["model_provider"].as_str(),
            Some("openai")
        );
        assert_eq!(lines[0]["payload"]["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(lines[1]["payload"]["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(
            lines[1]["payload"]["collaboration_mode"]["settings"]["model"].as_str(),
            Some("gpt-5.5")
        );
        assert_eq!(
            lines[2]["payload"]["thread_settings"]["model_provider_id"].as_str(),
            Some("openai")
        );
        assert_eq!(
            lines[2]["payload"]["thread_settings"]["model"].as_str(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn catalog_visibility_only_reports_explicit_catalog_entries() {
        let mut catalog = std::collections::HashMap::new();
        catalog.insert(
            "visible-task".to_string(),
            ("Task".to_string(), String::new(), true),
        );
        catalog.insert(
            "removed-task".to_string(),
            ("Task".to_string(), String::new(), false),
        );
        let catalog = Some(catalog);
        assert_eq!(
            super::catalog_visibility(&catalog, "visible-task"),
            Some(true)
        );
        assert_eq!(
            super::catalog_visibility(&catalog, "removed-task"),
            Some(false)
        );
        assert_eq!(super::catalog_visibility(&catalog, "not-in-sidebar"), None);
    }

    #[test]
    fn desktop_sidebar_visibility_requires_a_current_sidebar_registration() {
        assert!(super::desktop_sidebar_visible(false, true));
        assert!(!super::desktop_sidebar_visible(false, false));
        assert!(!super::desktop_sidebar_visible(true, true));
    }

    #[test]
    fn dangling_project_assignment_is_not_sidebar_visible() {
        assert!(!super::desktop_sidebar_visible(false, false));
    }

    #[test]
    fn register_threads_writes_custom_model_metadata() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-db-metadata-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&home).ok();
        fs::create_dir_all(&home).unwrap();
        let connection = Connection::open(home.join("state_5.sqlite")).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    source TEXT NOT NULL,
                    model_provider TEXT NOT NULL,
                    cwd TEXT NOT NULL,
                    title TEXT NOT NULL,
                    sandbox_policy TEXT NOT NULL,
                    approval_mode TEXT NOT NULL,
                    git_branch TEXT,
                    git_origin_url TEXT,
                    first_user_message TEXT NOT NULL DEFAULT '',
                    memory_mode TEXT NOT NULL DEFAULT 'enabled',
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy',
                    has_user_event INTEGER NOT NULL DEFAULT 0,
                    archived INTEGER NOT NULL DEFAULT 0,
                    archived_at INTEGER,
                    cli_version TEXT NOT NULL DEFAULT '',
                    model TEXT,
                    reasoning_effort TEXT,
                    thread_source TEXT,
                    agent_path TEXT,
                    agent_nickname TEXT,
                    agent_role TEXT,
                    created_at_ms INTEGER,
                    updated_at_ms INTEGER
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let mut task = task_with("custom", "/tmp/project", "");
        task.id = "custom-model-task".to_string();
        task.created_at = "2026-07-19T12:00:00Z".to_string();
        task.updated_at = "2026-07-19T12:10:00Z".to_string();
        task.file_path = PathBuf::from("/tmp/custom-model-task.jsonl");
        task.source = "vscode".to_string();
        task.model_provider = "custom".to_string();
        task.model = "glm-5.2".to_string();
        task.reasoning_effort = "high".to_string();
        task.sandbox_policy = r#"{"type":"danger-full-access"}"#.to_string();
        task.approval_mode = "never".to_string();
        task.cli_version = "0.145.0-alpha.18".to_string();
        task.thread_source = "user".to_string();

        register_threads(&home, &[task]).unwrap();
        let connection = Connection::open(home.join("state_5.sqlite")).unwrap();
        let row = connection
            .query_row(
                "SELECT model_provider, model, reasoning_effort, cli_version, thread_source, sandbox_policy, approval_mode FROM threads WHERE id = 'custom-model-task'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .unwrap();
        drop(connection);
        fs::remove_dir_all(&home).ok();

        assert_eq!(row.0, "custom");
        assert_eq!(row.1, "glm-5.2");
        assert_eq!(row.2, "high");
        assert_eq!(row.3, "0.145.0-alpha.18");
        assert_eq!(row.4, "user");
        assert_eq!(row.5, r#"{"type":"danger-full-access"}"#);
        assert_eq!(row.6, "never");
    }

    #[test]
    fn register_catalog_threads_marks_existing_local_catalog_ready() {
        let home = env::temp_dir().join(format!(
            "codex-session-transfer-catalog-ready-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&home).ok();
        let sqlite = home.join("sqlite");
        fs::create_dir_all(&sqlite).unwrap();
        let connection = Connection::open(sqlite.join("codex-dev.db")).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT NOT NULL);
                CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER NOT NULL);
                CREATE TABLE local_thread_catalog_sync_state (host_id TEXT PRIMARY KEY, observation_sequence INTEGER NOT NULL, initial_build_complete INTEGER NOT NULL);
                CREATE TABLE local_thread_catalog (
                    host_id TEXT NOT NULL,
                    thread_id TEXT NOT NULL,
                    display_title TEXT NOT NULL,
                    source_created_at REAL NOT NULL,
                    source_updated_at REAL NOT NULL,
                    cwd TEXT NOT NULL,
                    source_kind TEXT NOT NULL,
                    source_detail TEXT,
                    model_provider TEXT NOT NULL,
                    git_branch TEXT,
                    observation_sequence INTEGER NOT NULL,
                    missing_candidate INTEGER NOT NULL,
                    PRIMARY KEY (host_id, thread_id)
                );
                INSERT INTO local_thread_catalog_hosts (host_id, host_kind) VALUES ('local', 'local');
                INSERT INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, 0);
                INSERT INTO local_thread_catalog_sync_state (host_id, observation_sequence, initial_build_complete) VALUES ('local', 41, 0);
                "#,
            )
            .unwrap();
        drop(connection);

        let mut task = task_with("catalog task", "/tmp/project", "");
        task.id = "catalog-ready-task".to_string();
        task.created_at = "2026-07-24T00:00:00Z".to_string();
        task.updated_at = "2026-07-24T00:01:00Z".to_string();
        task.source = "cli".to_string();
        task.model_provider = "openai".to_string();
        register_catalog_threads(&home, &[task]).unwrap();

        let connection = Connection::open(sqlite.join("codex-dev.db")).unwrap();
        let state = connection
            .query_row(
                "SELECT observation_sequence, initial_build_complete FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        drop(connection);
        fs::remove_dir_all(&home).ok();

        assert_eq!(state, (41, 1));
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
            model: String::new(),
            reasoning_effort: String::new(),
            sandbox_policy: String::new(),
            approval_mode: String::new(),
            cli_version: String::new(),
            thread_source: String::new(),
            forked_from_id: String::new(),
            agent_path: String::new(),
            agent_nickname: String::new(),
            agent_role: String::new(),
            memory_mode: String::new(),
            history_mode: String::new(),
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
    fn path_name_handles_windows_separators() {
        assert_eq!(
            super::path_name(r"E:\github项目集合\codex-session-transfer"),
            "codex-session-transfer"
        );
    }

    #[test]
    fn sort_tasks_by_project_order_keeps_source_sidebar_order() {
        let mut first = task_with("first", "/tmp/first", "");
        first.id = "first-task".to_string();
        first.project_key = "project-first".to_string();
        let mut second = task_with("second", "/tmp/second", "");
        second.id = "second-task".to_string();
        second.project_key = "project-second".to_string();
        let mut another_second = task_with("another", "/tmp/second", "");
        another_second.id = "another-second-task".to_string();
        another_second.project_key = "project-second".to_string();
        let mut tasks = vec![first, second, another_second];

        sort_tasks_by_project_order(
            &mut tasks,
            &["project-second".to_string(), "project-first".to_string()],
        );

        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["second-task", "another-second-task", "first-task"]
        );
    }

    #[test]
    fn missing_codex_worktree_stays_unbound_on_import() {
        let cwd = r"Z:\historical\.codex\worktrees\77c3\RAGTest";
        assert!(super::resolved_import_cwd(cwd, None, true).is_empty());
        assert!(super::resolved_import_cwd(cwd, Some(r"Z:\target"), true).is_empty());
    }

    #[test]
    fn resolve_local_cwd_falls_back_to_home_work_for_missing_path() {
        let name = format!("codex-session-transfer-missing-{}", std::process::id());
        let original = format!(r"Z:\old-drive\{name}");
        let resolved = super::resolve_local_cwd(&original);

        assert_ne!(resolved, original);
        assert!(
            resolved.ends_with(&format!("work/{name}"))
                || resolved.ends_with(&format!(r"work\{name}"))
        );
    }

    #[test]
    fn session_details_ignores_codex_context_as_user_title_seed() {
        let path = env::temp_dir().join(format!(
            "codex-session-transfer-context-title-{}.jsonl",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"context-title-task","timestamp":"2026-07-22T12:00:00Z","cwd":"E:\\github项目集合","source":"vscode"}}
{"type":"event_msg","payload":{"type":"user_message","message":"<environment_context> <cwd>E:\\github项目集合</cwd> <shell>powershell</shell> </environment_context>"}}
{"type":"event_msg","payload":{"type":"user_message","message":"请帮我检查项目导入逻辑"}}"#,
        )
        .unwrap();
        let task = session_details(&path).unwrap();
        fs::remove_file(&path).ok();

        assert_eq!(task.first_user_message, "请帮我检查项目导入逻辑");
        assert!(!task.first_user_message.contains("<cwd>"));
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
