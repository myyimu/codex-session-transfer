import { createReadStream, existsSync } from "node:fs";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  stat,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import AdmZip from "adm-zip";
import Database from "better-sqlite3";

const ARCHIVE_SCHEMA = "codex-session-transfer/v1";
const MAX_ARCHIVE_BYTES = 1024 * 1024 * 1024;

export function getCodexHome() {
  return path.resolve(process.env.CODEX_HOME || path.join(os.homedir(), ".codex"));
}

function parseJson(value, fallback = null) {
  try {
    return JSON.parse(value);
  } catch {
    return fallback;
  }
}

function cleanText(value) {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function truncate(value, limit = 96) {
  const text = cleanText(value);
  return text.length > limit ? `${text.slice(0, limit - 1)}…` : text;
}

function contentText(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => {
      if (!part || typeof part !== "object") return "";
      return part.text || part.input_text || part.output_text || "";
    })
    .filter(Boolean)
    .join("\n");
}

function messageFromRecord(record) {
  const payload = record?.payload;
  if (!payload || typeof payload !== "object") return null;

  if (payload.type === "message" && ["user", "assistant"].includes(payload.role)) {
    return { role: payload.role, text: contentText(payload.content) };
  }

  if (payload.type === "user_message") {
    return { role: "user", text: payload.message || payload.text || "" };
  }

  return null;
}

async function walkJsonlFiles(root) {
  if (!existsSync(root)) return [];
  const files = [];
  const pending = [root];
  while (pending.length) {
    const current = pending.pop();
    const entries = await readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const target = path.join(current, entry.name);
      if (entry.isDirectory()) pending.push(target);
      if (entry.isFile() && entry.name.endsWith(".jsonl")) files.push(target);
    }
  }
  return files;
}

async function readSessionIndex(codexHome) {
  const indexPath = path.join(codexHome, "session_index.jsonl");
  if (!existsSync(indexPath)) return new Map();
  const lines = (await readFile(indexPath, "utf8")).split(/\r?\n/);
  const result = new Map();
  for (const line of lines) {
    const item = parseJson(line);
    if (item?.id) result.set(item.id, item);
  }
  return result;
}

function readSqliteRows(dbPath, query) {
  if (!existsSync(dbPath)) return [];
  try {
    const db = new Database(dbPath, { readonly: true, fileMustExist: true });
    const rows = db.prepare(query).all();
    db.close();
    return rows;
  } catch {
    return [];
  }
}

function threadDatabaseRows(codexHome) {
  return readSqliteRows(
    path.join(codexHome, "state_5.sqlite"),
    "SELECT id, title, rollout_path, created_at, updated_at, cwd, archived, git_branch, git_origin_url, preview, source, model_provider FROM threads",
  );
}

function catalogRows(codexHome) {
  return readSqliteRows(
    path.join(codexHome, "sqlite", "codex-dev.db"),
    "SELECT thread_id AS id, display_title AS title, source_created_at AS created_at, source_updated_at AS updated_at, cwd, git_branch FROM local_thread_catalog WHERE missing_candidate = 0",
  );
}

export async function parseSessionFile(filePath) {
  const details = {
    id: "",
    createdAt: "",
    cwd: "",
    source: "",
    modelProvider: "",
    gitBranch: "",
    gitOriginUrl: "",
    firstUserMessage: "",
    preview: "",
    messageCount: 0,
    userMessageCount: 0,
  };

  const input = createReadStream(filePath, { encoding: "utf8" });
  const lines = readline.createInterface({ input, crlfDelay: Infinity });

  for await (const line of lines) {
    const record = parseJson(line);
    if (!record) continue;
    if (record.type === "session_meta") {
      const payload = record.payload || {};
      details.id = payload.id || details.id;
      details.createdAt = payload.timestamp || record.timestamp || details.createdAt;
      details.cwd = payload.cwd || details.cwd;
      details.source = payload.source || details.source;
      details.modelProvider = payload.model_provider || details.modelProvider;
      details.gitBranch = payload.git?.branch || details.gitBranch;
      details.gitOriginUrl = payload.git?.repository_url || details.gitOriginUrl;
    }

    const message = messageFromRecord(record);
    if (!message || !cleanText(message.text)) continue;
    details.messageCount += 1;
    if (message.role === "user") {
      details.userMessageCount += 1;
      if (!details.firstUserMessage) details.firstUserMessage = cleanText(message.text);
      details.preview = cleanText(message.text);
    }
  }

  const fileStats = await stat(filePath);
  return { ...details, filePath, size: fileStats.size, modifiedAt: fileStats.mtime.toISOString() };
}

export async function listCodexTasks() {
  const codexHome = getCodexHome();
  const [index, dbRows, catalog, scannedFiles] = await Promise.all([
    readSessionIndex(codexHome),
    Promise.resolve(threadDatabaseRows(codexHome)),
    Promise.resolve(catalogRows(codexHome)),
    walkJsonlFiles(path.join(codexHome, "sessions")),
  ]);

  const dbById = new Map(dbRows.map((row) => [row.id, row]));
  const catalogById = new Map(catalog.map((row) => [row.id, row]));
  const fileSet = new Set(scannedFiles);
  for (const row of dbRows) {
    if (row.rollout_path && existsSync(row.rollout_path)) fileSet.add(row.rollout_path);
  }

  const parsed = await Promise.all([...fileSet].map((file) => parseSessionFile(file).catch(() => null)));
  const tasks = parsed
    .filter((item) => item?.id)
    .map((session) => {
      const indexed = index.get(session.id) || {};
      const db = dbById.get(session.id) || {};
      const catalogItem = catalogById.get(session.id) || {};
      const title =
        indexed.thread_name ||
        db.title ||
        catalogItem.title ||
        truncate(session.firstUserMessage) ||
        `未命名任务 ${session.id.slice(0, 8)}`;
      const secondsToIso = (value) =>
        value ? new Date(Number(value) * 1000).toISOString() : "";

      return {
        ...session,
        title,
        createdAt: session.createdAt || secondsToIso(db.created_at || catalogItem.created_at),
        updatedAt:
          indexed.updated_at ||
          secondsToIso(db.updated_at || catalogItem.updated_at) ||
          session.modifiedAt,
        cwd: session.cwd || db.cwd || catalogItem.cwd || "",
        archived: Boolean(db.archived),
        gitBranch: session.gitBranch || db.git_branch || catalogItem.git_branch || "",
        gitOriginUrl: session.gitOriginUrl || db.git_origin_url || "",
        preview: db.preview || session.preview || session.firstUserMessage,
        source: session.source || db.source || "vscode",
        modelProvider: session.modelProvider || db.model_provider || "openai",
        browserFile: path.join(codexHome, "browser", "sessions", `${session.id}.toml`),
      };
    })
    .sort((a, b) => new Date(b.updatedAt || 0) - new Date(a.updatedAt || 0));

  return { tasks, codexHome };
}

function archiveTaskPath(id, name) {
  return `tasks/${id}/${name}`;
}

function validateManifest(manifest) {
  if (manifest?.schema !== ARCHIVE_SCHEMA || !Array.isArray(manifest.tasks)) {
    throw new Error("这不是有效的 Codex 会话迁移压缩包");
  }
  for (const task of manifest.tasks) {
    if (!/^[a-zA-Z0-9-]{8,}$/.test(task.id || "")) {
      throw new Error("压缩包包含无效的任务标识");
    }
    if (!task.sessionFile?.startsWith(`tasks/${task.id}/`)) {
      throw new Error("压缩包目录结构不安全");
    }
  }
  return manifest;
}

export async function exportTaskArchive(taskIds, destination) {
  const requested = new Set(taskIds || []);
  const { tasks } = await listCodexTasks();
  const selected = tasks.filter((task) => requested.has(task.id));
  if (!selected.length) throw new Error("请至少选择一个任务");

  const zip = new AdmZip();
  const manifestTasks = [];
  for (const task of selected) {
    const sessionFile = archiveTaskPath(task.id, "session.jsonl");
    zip.addLocalFile(task.filePath, path.posix.dirname(sessionFile), path.posix.basename(sessionFile));
    const browserExists = existsSync(task.browserFile);
    const browserFile = browserExists ? archiveTaskPath(task.id, "browser.toml") : null;
    if (browserExists) {
      zip.addLocalFile(task.browserFile, path.posix.dirname(browserFile), path.posix.basename(browserFile));
    }
    manifestTasks.push({
      id: task.id,
      title: task.title,
      createdAt: task.createdAt,
      updatedAt: task.updatedAt,
      cwd: task.cwd,
      source: task.source,
      modelProvider: task.modelProvider,
      gitBranch: task.gitBranch,
      gitOriginUrl: task.gitOriginUrl,
      preview: task.preview,
      firstUserMessage: task.firstUserMessage,
      messageCount: task.messageCount,
      userMessageCount: task.userMessageCount,
      sessionFile,
      browserFile,
    });
  }

  const manifest = {
    schema: ARCHIVE_SCHEMA,
    createdAt: new Date().toISOString(),
    sourcePlatform: process.platform,
    tasks: manifestTasks,
  };
  zip.addFile("manifest.json", Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8"));
  await mkdir(path.dirname(destination), { recursive: true });
  zip.writeZip(destination);
  const archiveStats = await stat(destination);
  return { canceled: false, path: destination, count: selected.length, size: archiveStats.size };
}

function readArchive(archivePath) {
  if (!existsSync(archivePath)) throw new Error("找不到所选压缩包");
  const zip = new AdmZip(archivePath);
  const entries = zip.getEntries();
  const totalSize = entries.reduce((sum, entry) => sum + Number(entry.header.size || 0), 0);
  if (totalSize > MAX_ARCHIVE_BYTES) throw new Error("压缩包解压后超过 1 GB，已停止导入");
  const manifestEntry = zip.getEntry("manifest.json");
  if (!manifestEntry) throw new Error("压缩包缺少 manifest.json");
  const manifest = validateManifest(parseJson(manifestEntry.getData().toString("utf8")));
  for (const task of manifest.tasks) {
    if (!zip.getEntry(task.sessionFile)) throw new Error(`任务 ${task.title || task.id} 缺少会话文件`);
    if (task.browserFile && !zip.getEntry(task.browserFile)) {
      throw new Error(`任务 ${task.title || task.id} 缺少浏览器配置`);
    }
  }
  return { zip, manifest };
}

export async function inspectTaskArchive(archivePath) {
  const { manifest } = readArchive(archivePath);
  const existing = new Set((await listCodexTasks()).tasks.map((task) => task.id));
  return {
    canceled: false,
    path: archivePath,
    createdAt: manifest.createdAt,
    tasks: manifest.tasks.map((task) => ({ ...task, conflict: existing.has(task.id) })),
  };
}

function pathBasename(input) {
  return path.win32.basename(input || "") || path.posix.basename(input || "");
}

function resolveLocalCwd(originalCwd) {
  if (!originalCwd || existsSync(originalCwd)) return originalCwd;
  const name = pathBasename(originalCwd.replace(/[\\/]+$/, ""));
  const candidates = [
    path.join(os.homedir(), "work", name),
    path.join(os.homedir(), "Projects", name),
    path.join(os.homedir(), "Documents", name),
  ];
  return candidates.find((candidate) => existsSync(candidate)) || originalCwd;
}

function replaceStringsDeep(value, from, to) {
  if (!from || from === to) return value;
  if (typeof value === "string") return value.split(from).join(to);
  if (Array.isArray(value)) return value.map((item) => replaceStringsDeep(item, from, to));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, replaceStringsDeep(item, from, to)]),
    );
  }
  return value;
}

export function rewriteSessionCwd(content, from, to) {
  if (!from || !to || from === to) return content;
  return content
    .split(/\r?\n/)
    .filter((line, index, lines) => line || index < lines.length - 1)
    .map((line) => {
      if (!line) return line;
      const parsed = parseJson(line);
      return parsed ? JSON.stringify(replaceStringsDeep(parsed, from, to)) : line.split(from).join(to);
    })
    .join("\n");
}

async function appendSessionIndex(codexHome, tasks) {
  const indexPath = path.join(codexHome, "session_index.jsonl");
  const existing = existsSync(indexPath) ? await readFile(indexPath, "utf8") : "";
  const lines = existing.split(/\r?\n/).filter(Boolean);
  const ids = new Set(lines.map((line) => parseJson(line)?.id).filter(Boolean));
  for (const task of tasks) {
    if (ids.has(task.id)) continue;
    lines.push(JSON.stringify({ id: task.id, thread_name: task.title, updated_at: task.updatedAt }));
  }
  await writeFile(indexPath, `${lines.join("\n")}\n`, "utf8");
}

async function backupDatabase(dbPath, stamp) {
  if (!existsSync(dbPath)) return null;
  const backupPath = `${dbPath}.backup-${stamp}`;
  const db = new Database(dbPath, { fileMustExist: true });
  await db.backup(backupPath);
  db.close();
  return backupPath;
}

function registerStateThreads(codexHome, tasks) {
  const dbPath = path.join(codexHome, "state_5.sqlite");
  if (!existsSync(dbPath)) return;
  const db = new Database(dbPath);
  const insert = db.prepare(`
    INSERT OR IGNORE INTO threads (
      id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
      sandbox_policy, approval_mode, git_branch, git_origin_url, first_user_message,
      memory_mode, preview, recency_at, recency_at_ms, history_mode, has_user_event
    ) VALUES (
      @id, @rolloutPath, @createdAt, @updatedAt, @source, @modelProvider, @cwd, @title,
      '{"type":"disabled"}', 'never', @gitBranch, @gitOriginUrl, @firstUserMessage,
      'enabled', @preview, @updatedAt, @updatedAtMs, 'legacy', 1
    )
  `);
  const transaction = db.transaction((items) => items.forEach((item) => insert.run(item)));
  transaction(tasks);
  db.close();
}

function registerCatalogThreads(codexHome, tasks) {
  const dbPath = path.join(codexHome, "sqlite", "codex-dev.db");
  if (!existsSync(dbPath)) return;
  const db = new Database(dbPath);
  const hasCatalog = db.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_thread_catalog'").get();
  if (!hasCatalog) {
    db.close();
    return;
  }
  const current = db.prepare("SELECT COALESCE(MAX(observation_sequence), 0) AS value FROM local_thread_catalog").get().value;
  const insert = db.prepare(`
    INSERT OR IGNORE INTO local_thread_catalog (
      host_id, thread_id, display_title, source_created_at, source_updated_at, cwd,
      source_kind, source_detail, model_provider, git_branch, observation_sequence, missing_candidate
    ) VALUES ('local', @id, @title, @createdAt, @updatedAt, @cwd,
      @source, NULL, @modelProvider, @gitBranch, @sequence, 0)
  `);
  const transaction = db.transaction((items) => {
    items.forEach((item, index) => insert.run({ ...item, sequence: current + index + 1 }));
    db.prepare("UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + 1 WHERE id = 1").run();
    db.prepare("UPDATE local_thread_catalog_sync_state SET observation_sequence = MAX(observation_sequence, ?) WHERE host_id = 'local'").run(current + items.length);
  });
  transaction(tasks);
  db.close();
}

export async function importTaskArchive(archivePath, options = {}) {
  const { zip, manifest } = readArchive(archivePath);
  const codexHome = getCodexHome();
  await mkdir(path.join(codexHome, "sessions"), { recursive: true });
  await mkdir(path.join(codexHome, "browser", "sessions"), { recursive: true });
  const existing = new Set((await listCodexTasks()).tasks.map((task) => task.id));
  const imported = [];
  const skipped = [];

  for (const task of manifest.tasks) {
    if (existing.has(task.id)) {
      skipped.push({ id: task.id, title: task.title, reason: "already_exists" });
      continue;
    }
    const created = new Date(task.createdAt || task.updatedAt || Date.now());
    const year = String(created.getFullYear());
    const month = String(created.getMonth() + 1).padStart(2, "0");
    const day = String(created.getDate()).padStart(2, "0");
    const targetDir = path.join(codexHome, "sessions", year, month, day);
    await mkdir(targetDir, { recursive: true });
    const timestamp = created.toISOString().replace(/[:.]/g, "-").replace("Z", "");
    const rolloutPath = path.join(targetDir, `rollout-${timestamp}-${task.id}.jsonl`);
    const sourceContent = zip.getEntry(task.sessionFile).getData().toString("utf8");
    const localCwd = options.adaptPaths === false ? task.cwd : resolveLocalCwd(task.cwd);
    const content = rewriteSessionCwd(sourceContent, task.cwd, localCwd);
    await writeFile(rolloutPath, content.endsWith("\n") ? content : `${content}\n`, "utf8");

    if (task.browserFile) {
      const browserPath = path.join(codexHome, "browser", "sessions", `${task.id}.toml`);
      await writeFile(browserPath, zip.getEntry(task.browserFile).getData());
    }

    const createdAt = Math.floor(created.getTime() / 1000);
    const updatedDate = new Date(task.updatedAt || task.createdAt || Date.now());
    const updatedAt = Math.floor(updatedDate.getTime() / 1000);
    imported.push({
      ...task,
      cwd: localCwd || task.cwd || os.homedir(),
      rolloutPath,
      createdAt,
      updatedAt,
      updatedAtMs: updatedAt * 1000,
      source: task.source || "vscode",
      modelProvider: task.modelProvider || "openai",
      title: task.title || truncate(task.firstUserMessage) || `导入任务 ${task.id.slice(0, 8)}`,
      preview: task.preview || task.firstUserMessage || task.title || "",
      firstUserMessage: task.firstUserMessage || task.preview || task.title || "",
      gitBranch: task.gitBranch || null,
      gitOriginUrl: task.gitOriginUrl || null,
      indexUpdatedAt: updatedDate.toISOString(),
    });
  }

  if (!imported.length) return { imported: [], skipped, backups: [], codexHome };

  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const backups = (await Promise.all([
    backupDatabase(path.join(codexHome, "state_5.sqlite"), stamp),
    backupDatabase(path.join(codexHome, "sqlite", "codex-dev.db"), stamp),
  ])).filter(Boolean);

  await appendSessionIndex(
    codexHome,
    imported.map((task) => ({
      id: task.id,
      title: task.title,
      updatedAt: task.indexUpdatedAt,
    })),
  );
  registerStateThreads(codexHome, imported);
  registerCatalogThreads(codexHome, imported);

  return {
    imported: imported.map(({ id, title, cwd, rolloutPath }) => ({ id, title, cwd, rolloutPath })),
    skipped,
    backups,
    codexHome,
  };
}

