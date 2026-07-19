import { useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  ArrowClockwise,
  CaretDown,
  Check,
  CheckSquare,
  Clock,
  CloudArrowDown,
  DownloadSimple,
  Export,
  FileArchive,
  FolderOpen,
  FolderSimple,
  MagnifyingGlass,
  Path,
  ShieldCheck,
  TrayArrowDown,
  X,
} from "@phosphor-icons/react";
import { demoBridge } from "./demoBridge.js";
import { tauriBridge } from "./tauriBridge.js";

const bridge = window.__TAURI_INTERNALS__ ? tauriBridge : demoBridge;

function formatDate(value) {
  if (!value) return "时间未知";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "时间未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function formatBytes(value) {
  if (!Number.isFinite(value)) return "";
  if (value < 1024 * 1024) return `${Math.max(1, Math.round(value / 1024))} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

function filename(value) {
  return String(value || "").split(/[\\/]/).pop();
}

function projectKey(task) {
  return task.projectKey || task.cwd || "__unknown__";
}

function projectName(task) {
  const key = projectKey(task);
  if (task.projectName) return task.projectName;
  if (!key || key === "__unknown__") return "未记录项目";
  return filename(key) || key;
}

function projectMissing(task) {
  return Boolean(task.cwd) && task.projectExists === false;
}

function buildProjects(tasks) {
  const groups = new Map();
  tasks.forEach((task) => {
    const key = projectKey(task);
    const current = groups.get(key) || {
      key,
      name: projectName(task),
      path: task.projectPath || task.cwd || "",
      count: 0,
      missing: false,
      updatedAt: "",
    };
    current.count += 1;
    current.missing ||= projectMissing(task);
    if (!current.path && task.cwd) current.path = task.cwd;
    if ((task.updatedAt || "") > current.updatedAt) current.updatedAt = task.updatedAt || "";
    groups.set(key, current);
  });
  return [...groups.values()].sort((left, right) => {
    if (left.missing !== right.missing) return left.missing ? 1 : -1;
    if (right.count !== left.count) return right.count - left.count;
    return left.name.localeCompare(right.name, "zh-CN");
  });
}

function Toast({ toast, onClose }) {
  if (!toast) return null;
  return (
    <div className={`toast toast-${toast.type}`} role="status">
      <span className="toast-icon"><Check size={16} weight="bold" /></span>
      <div>
        <strong>{toast.title}</strong>
        {toast.message && <p>{toast.message}</p>}
      </div>
      <button className="icon-button" onClick={onClose} aria-label="关闭提示" title="关闭提示">
        <X size={16} />
      </button>
    </div>
  );
}

function TaskRow({ task, selected, onToggle }) {
  return (
    <label className={`task-row ${selected ? "is-selected" : ""}`}>
      <input type="checkbox" checked={selected} onChange={() => onToggle(task.id)} />
      <span className="check-control" aria-hidden="true">
        {selected && <Check size={13} weight="bold" />}
      </span>
      <span className="task-mark"><Archive size={20} weight="duotone" /></span>
      <span className="task-copy">
        <span className="task-title-line">
          <strong>{task.title}</strong>
          {task.archived && <span className="status-chip neutral">已归档</span>}
          {projectMissing(task) && <span className="status-chip warning">项目已删除</span>}
        </span>
        <span className="task-meta">
          <span><Clock size={13} />{formatDate(task.updatedAt)}</span>
          <span><Path size={13} />{task.cwd || "未记录工作目录"}</span>
        </span>
      </span>
      <span className="task-stats">
        <strong>{task.userMessageCount || 0}</strong>
        <span>轮对话</span>
      </span>
    </label>
  );
}

function ProjectPicker({ tasks, projects, value, onChange }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);
  const selectedProject = projects.find((item) => item.key === value);
  const label = selectedProject
    ? `${selectedProject.name} · ${selectedProject.missing ? "已删除 · " : ""}${selectedProject.count}`
    : `全部项目 · ${tasks.length}`;

  useEffect(() => {
    if (!open) return undefined;
    const close = (event) => {
      if (!rootRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const choose = (next) => {
    onChange(next);
    setOpen(false);
  };

  return (
    <div className={`project-picker ${open ? "is-open" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="project-trigger"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-haspopup="listbox"
        title="按项目筛选任务"
      >
        <FolderSimple size={18} weight="duotone" />
        <span>{label}</span>
        <CaretDown size={14} weight="bold" />
      </button>
      {open && (
        <div className="project-menu" role="listbox" aria-label="按项目筛选">
          <button
            type="button"
            className={`project-option ${value === "all" ? "is-active" : ""}`}
            onClick={() => choose("all")}
            role="option"
            aria-selected={value === "all"}
          >
            <span>全部项目</span>
            <strong>{tasks.length}</strong>
          </button>
          {projects.map((item) => (
            <button
              key={item.key}
              type="button"
              className={`project-option ${item.missing ? "is-missing" : ""} ${value === item.key ? "is-active" : ""}`}
              onClick={() => choose(item.key)}
              role="option"
              aria-selected={value === item.key}
            >
              <span>{item.name}</span>
              {item.missing && <em>已删除</em>}
              <strong>{item.count}</strong>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function ImportProjectPicker({ projects, value, onChange }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);
  const selected = projects.find((item) => item.path === value);
  const label = selected ? selected.name : "保持压缩包中的项目";

  useEffect(() => {
    if (!open) return undefined;
    const close = (event) => {
      if (!rootRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const choose = (next) => {
    onChange(next);
    setOpen(false);
  };

  return (
    <div className={`project-picker import-project-picker ${open ? "is-open" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="project-trigger"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-haspopup="listbox"
        title="选择恢复到哪个项目"
      >
        <FolderSimple size={18} weight="duotone" />
        <span>{label}</span>
        <CaretDown size={14} weight="bold" />
      </button>
      {open && (
        <div className="project-menu" role="listbox" aria-label="选择目标项目">
          <button
            type="button"
            className={`project-option ${value ? "" : "is-active"}`}
            onClick={() => choose("")}
            role="option"
            aria-selected={!value}
          >
            <span>保持压缩包中的项目</span>
            <strong>默认</strong>
          </button>
          {projects.map((item) => (
            <button
              key={item.key}
              type="button"
              className={`project-option ${value === item.path ? "is-active" : ""}`}
              onClick={() => choose(item.path)}
              role="option"
              aria-selected={value === item.path}
            >
              <span>{item.name}</span>
              <strong>{item.count}</strong>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function NavButton({ active, icon: Icon, title, subtitle, onSelect }) {
  const handlePointerDown = (event) => {
    if (event.button === undefined || event.button === 0) {
      event.preventDefault();
      onSelect();
    }
  };

  return (
    <button
      type="button"
      className={active ? "active" : ""}
      onClick={onSelect}
      onPointerDown={handlePointerDown}
    >
      <Icon size={20} weight={active ? "fill" : "regular"} />
      <span><strong>{title}</strong><small>{subtitle}</small></span>
    </button>
  );
}

function ExportView({ environment, showToast }) {
  const [tasks, setTasks] = useState([]);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("all");
  const [selected, setSelected] = useState(new Set());
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      const result = await bridge.listTasks();
      setTasks(result.tasks || []);
    } catch (error) {
      showToast("error", "读取失败", error.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, []);

  const projects = useMemo(() => buildProjects(tasks), [tasks]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return tasks.filter((task) => {
      if (project !== "all" && projectKey(task) !== project) return false;
      if (!needle) return true;
      return [task.title, task.cwd, task.preview, task.gitBranch]
        .some((value) => String(value || "").toLocaleLowerCase().includes(needle));
    });
  }, [project, query, tasks]);

  const selectedTasks = tasks.filter((task) => selected.has(task.id));
  const selectedBytes = selectedTasks.reduce((sum, task) => sum + (task.size || 0), 0);
  const allVisibleSelected = filtered.length > 0 && filtered.every((task) => selected.has(task.id));

  const toggle = (id) => {
    setSelected((current) => {
      const next = new Set(current);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };

  const toggleVisible = () => {
    setSelected((current) => {
      const next = new Set(current);
      if (allVisibleSelected) filtered.forEach((task) => next.delete(task.id));
      else filtered.forEach((task) => next.add(task.id));
      return next;
    });
  };

  const exportSelected = async () => {
    setWorking(true);
    try {
      const result = await bridge.exportTasks([...selected]);
      if (!result.canceled) {
        showToast("success", `已打包 ${result.count} 个任务`, filename(result.path));
      }
    } catch (error) {
      showToast("error", "导出失败", error.message);
    } finally {
      setWorking(false);
    }
  };

  return (
    <section className="workspace" aria-labelledby="export-title">
      <header className="workspace-header">
        <div>
          <span className="eyebrow">从这台电脑导出</span>
          <h1 id="export-title">选择要带走的任务</h1>
          <p>{loading ? "正在读取…" : `${tasks.length} 个 Codex 任务 · 数据只在本机处理`}</p>
        </div>
        <button className="icon-button refresh-button" onClick={load} disabled={loading} title="重新扫描" aria-label="重新扫描">
          <ArrowClockwise size={19} className={loading ? "spin" : ""} />
        </button>
      </header>

      <div className="toolbar">
        <div className="filter-row">
          <label className="search-field">
            <MagnifyingGlass size={18} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索任务名、路径或内容" />
            {query && (
              <button onClick={() => setQuery("")} aria-label="清空搜索" title="清空搜索"><X size={15} /></button>
            )}
          </label>
          <ProjectPicker tasks={tasks} projects={projects} value={project} onChange={setProject} />
        </div>
        <div className="toolbar-actions">
          <button className="text-button" onClick={toggleVisible} disabled={!filtered.length}>
            <CheckSquare size={17} weight={allVisibleSelected ? "fill" : "regular"} />
            {allVisibleSelected ? "取消全选" : "全选当前结果"}
          </button>
          <button className="primary-button toolbar-export" disabled={!selected.size || working} onClick={exportSelected}>
            <Export size={17} weight="bold" />
            {working ? "正在打包…" : selected.size ? `导出 ${selected.size} 个任务` : "导出已选任务"}
          </button>
        </div>
      </div>

      <div className="task-list" aria-live="polite">
        {loading && <div className="empty-state"><ArrowClockwise size={28} className="spin" /><strong>正在扫描 Codex 任务</strong></div>}
        {!loading && !filtered.length && (
          <div className="empty-state"><MagnifyingGlass size={28} /><strong>没有找到匹配的任务</strong><span>换一个关键词试试</span></div>
        )}
        {!loading && filtered.map((task) => (
          <TaskRow key={task.id} task={task} selected={selected.has(task.id)} onToggle={toggle} />
        ))}
      </div>

      <footer className="action-bar">
        <div>
          <strong>{selected.size ? `已选择 ${selected.size} 个任务` : "尚未选择任务"}</strong>
          <span>{selected.size ? `原始会话约 ${formatBytes(selectedBytes)}` : environment?.codexHome}</span>
        </div>
        <button className="primary-button" disabled={!selected.size || working} onClick={exportSelected}>
          <Export size={18} weight="bold" />
          {working ? "正在打包…" : "导出压缩包"}
        </button>
      </footer>
    </section>
  );
}

function ImportView({ environment, showToast }) {
  const [archive, setArchive] = useState(null);
  const [adaptPaths, setAdaptPaths] = useState(true);
  const [restoreExisting, setRestoreExisting] = useState(false);
  const [targetCwd, setTargetCwd] = useState("");
  const [localTasks, setLocalTasks] = useState([]);
  const [dragging, setDragging] = useState(false);
  const [working, setWorking] = useState(false);
  const [result, setResult] = useState(null);

  useEffect(() => {
    bridge.listTasks().then((next) => setLocalTasks(next.tasks || [])).catch(() => setLocalTasks([]));
  }, []);

  const choose = async () => {
    try {
      const next = await bridge.chooseArchive();
      if (!next.canceled) {
        setArchive(next);
        setResult(null);
        setRestoreExisting(false);
      }
    } catch (error) {
      showToast("error", "无法读取压缩包", error.message);
    }
  };

  const handleDrop = async (event) => {
    event.preventDefault();
    setDragging(false);
    const file = event.dataTransfer.files?.[0];
    if (!file) return;
    try {
      const archivePath = bridge.getPathForFile ? bridge.getPathForFile(file) : null;
      if (!archivePath) {
        showToast("error", "请使用选择文件", "Tauri 版本会通过系统文件选择器安全读取压缩包");
        return;
      }
      const next = await bridge.inspectArchive(archivePath);
      setArchive(next);
      setResult(null);
      setRestoreExisting(false);
    } catch (error) {
      showToast("error", "无法读取压缩包", error.message);
    }
  };

  const runImport = async () => {
    setWorking(true);
    try {
      const next = await bridge.importArchive(archive.path, { adaptPaths, restoreExisting, targetCwd });
      const importedCount = next.imported?.length || 0;
      const restoredCount = next.restored?.length || 0;
      const skippedCount = next.skipped?.length || 0;
      setResult(next);
      showToast(
        "success",
        importedCount || restoredCount ? `已处理 ${importedCount + restoredCount} 个任务` : "没有需要导入的任务",
        skippedCount ? `${skippedCount} 个重复任务已跳过` : "重启 Codex 后即可看到",
      );
    } catch (error) {
      showToast("error", "导入失败", error.message);
    } finally {
      setWorking(false);
    }
  };

  const importableCount = archive?.tasks.filter((task) => !task.conflict).length || 0;
  const conflictCount = archive?.tasks.filter((task) => task.conflict).length || 0;
  const actionCount = importableCount + (restoreExisting ? conflictCount : 0);
  const localProjects = useMemo(() => buildProjects(localTasks).filter((item) => !item.missing && item.path), [localTasks]);
  const targetName = targetCwd ? (localProjects.find((item) => item.path === targetCwd)?.name || filename(targetCwd)) : "";

  return (
    <section className="workspace import-workspace" aria-labelledby="import-title">
      <header className="workspace-header">
        <div>
          <span className="eyebrow rose">导入到这台电脑</span>
          <h1 id="import-title">恢复 Codex 任务</h1>
          <p>可跳过重复任务，也可从本地历史恢复到 Codex 列表</p>
        </div>
      </header>

      {!archive ? (
        <button
          className={`drop-zone ${dragging ? "is-dragging" : ""}`}
          onClick={choose}
          onDragEnter={() => setDragging(true)}
          onDragLeave={() => setDragging(false)}
          onDragOver={(event) => event.preventDefault()}
          onDrop={handleDrop}
        >
          <span className="drop-icon"><CloudArrowDown size={34} weight="duotone" /></span>
          <strong>拖入会话压缩包</strong>
          <span>或点击选择 `.zip` 文件</span>
        </button>
      ) : (
        <div className="import-content">
          <div className="archive-summary">
            <span className="archive-icon"><FileArchive size={26} weight="duotone" /></span>
            <span>
              <strong>{filename(archive.path)}</strong>
              <small>{archive.tasks.length} 个任务 · 打包于 {formatDate(archive.createdAt)}</small>
            </span>
            <button className="text-button compact" onClick={choose}>重新选择</button>
          </div>

          <div className="import-list">
            {archive.tasks.map((task) => (
              <div className="import-row" key={task.id}>
                <span className={`import-state ${task.conflict ? "skip" : "ready"}`}>
                  {task.conflict ? <ArrowClockwise size={15} /> : <Check size={15} weight="bold" />}
                </span>
                <span className="task-copy">
                  <strong>{task.title}</strong>
                  <span>{task.cwd || "未记录工作目录"}</span>
                </span>
                <span className={`status-chip ${task.conflict ? "neutral" : "ready"}`}>
                  {task.conflict ? (restoreExisting ? "将恢复" : "已存在，将跳过") : "可导入"}
                </span>
              </div>
            ))}
          </div>

          {conflictCount > 0 && (
            <div className="setting-row choice-row">
              <span className="setting-icon"><ArrowClockwise size={19} /></span>
              <span>
                <strong>重复任务处理</strong>
                <small>{conflictCount} 个任务本地历史已存在，但可重新恢复到 Codex 列表</small>
              </span>
              <span className="segmented-control" role="group" aria-label="重复任务处理">
                <button type="button" className={!restoreExisting ? "is-active" : ""} onClick={() => setRestoreExisting(false)}>跳过</button>
                <button type="button" className={restoreExisting ? "is-active" : ""} onClick={() => setRestoreExisting(true)}>从本地历史恢复</button>
              </span>
            </div>
          )}

          <div className="setting-row project-target-row">
            <span className="setting-icon"><FolderOpen size={19} /></span>
            <span>
              <strong>导入到项目</strong>
              <small>{targetCwd ? `恢复到 ${targetName || targetCwd}` : "默认保留压缩包记录的项目路径"}</small>
            </span>
            <ImportProjectPicker projects={localProjects} value={targetCwd} onChange={setTargetCwd} />
          </div>

          <label className="setting-row">
            <span className="setting-icon"><Path size={19} /></span>
            <span>
              <strong>自动适配项目路径</strong>
              <small>原路径不存在时，查找本机 `work`、`Projects` 和 `Documents`</small>
            </span>
            <input type="checkbox" checked={adaptPaths} onChange={(event) => setAdaptPaths(event.target.checked)} />
            <span className="switch" aria-hidden="true"><span /></span>
          </label>

          {result && (
            <div className="result-strip">
              <ShieldCheck size={20} weight="duotone" />
              <span><strong>导入完成</strong><small>{result.restored?.length ? "已从本地历史恢复，请重启 Codex 查看" : "请重启 Codex 刷新任务列表"}</small></span>
              <button className="icon-button" onClick={() => bridge.revealPath(environment.codexHome)} title="打开 Codex 数据目录" aria-label="打开 Codex 数据目录"><FolderOpen size={18} /></button>
            </div>
          )}
        </div>
      )}

      <footer className="action-bar">
        <div>
          <strong>{archive ? (restoreExisting ? `${importableCount} 个新任务，${conflictCount} 个可恢复` : `${importableCount} 个新任务可导入`) : "等待选择压缩包"}</strong>
          <span>{environment?.codexHome}</span>
        </div>
        <button className="primary-button rose-button" disabled={!archive || !actionCount || working} onClick={runImport}>
          <TrayArrowDown size={18} weight="bold" />
          {working ? "正在导入…" : "导入到 Codex"}
        </button>
      </footer>
    </section>
  );
}

export function App() {
  const [mode, setMode] = useState("export");
  const [environment, setEnvironment] = useState(null);
  const [toast, setToast] = useState(null);

  useEffect(() => {
    bridge.getEnvironment().then(setEnvironment);
  }, []);

  const showToast = (type, title, message) => {
    setToast({ type, title, message });
    window.setTimeout(() => setToast(null), 5000);
  };

  return (
    <main className="app-shell">
      <div className="window-drag-region" />
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark"><img src="/transfer-icon-dream.png" alt="" /></span>
          <span><strong>Codex 迁移</strong><small>Session Transfer</small></span>
        </div>

        <nav aria-label="主要功能">
          <NavButton
            active={mode === "export"}
            icon={DownloadSimple}
            title="导出任务"
            subtitle="打包这台电脑的会话"
            onSelect={() => setMode("export")}
          />
          <NavButton
            active={mode === "import"}
            icon={CloudArrowDown}
            title="导入任务"
            subtitle="恢复到新的电脑"
            onSelect={() => setMode("import")}
          />
        </nav>

        <div className="privacy-note">
          <ShieldCheck size={18} weight="duotone" />
          <span><strong>本地处理</strong><small>不会上传会话内容</small></span>
        </div>

        <div className="sidebar-footer">
          <span className="online-dot" />
          <span><strong>Codex 数据已连接</strong><small>v{environment?.version || "0.1.0"}</small></span>
        </div>
      </aside>

      {mode === "export" ? (
        <ExportView environment={environment} showToast={showToast} />
      ) : (
        <ImportView environment={environment} showToast={showToast} />
      )}
      <Toast toast={toast} onClose={() => setToast(null)} />
    </main>
  );
}
