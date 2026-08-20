import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  Wrench,
  X,
} from "@phosphor-icons/react";
import { demoBridge } from "./demoBridge.js";
import { tauriBridge } from "./tauriBridge.js";
import {
  activeProjectMappings,
  isCodexWorktreePath,
  normalizeDisplayPath,
  prepareProjectMappings,
  projectMappingStatus,
  repairableTaskIds,
} from "./taskLogic.js";

const bridge = window.__TAURI_INTERNALS__ ? tauriBridge : demoBridge;
const isMacOS = /Macintosh|Mac OS X/.test(navigator.userAgent);

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

function taskHiddenInCodex(task) {
  return Boolean(task.cwd) && task.codexVisible === false;
}

function modelLabel(task) {
  const provider = String(task.modelProvider || "").trim();
  const model = String(task.model || "").trim();
  if (provider && model) return `${provider} / ${model}`;
  return provider || model;
}

function compactPath(value) {
  const path = String(value || "");
  return path.replace(/^\/Users\/([^/]+)/, "~");
}

function historyDifferenceText(preview, source, label) {
  const isArchive = source === "archive";
  const recordCount = isArchive
    ? (preview?.archiveUniqueRecordCount || preview?.appendRecordCount || 0)
    : (preview?.localUniqueRecordCount || 0);
  const userTurnCount = isArchive
    ? preview?.archiveUniqueUserTurnCount
    : preview?.localUniqueUserTurnCount;
  const turnSummary = Number.isInteger(userTurnCount) ? `，约 ${userTurnCount} 轮对话` : "";
  return `${label || `${isArchive ? "归档" : "本地"}版本更完整`}：多 ${recordCount} 条会话记录${turnSummary}`;
}

function projectMatchesSearch(project, query) {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  return [project.displayName, project.name, project.path, project.key]
    .some((value) => String(value || "").toLocaleLowerCase().includes(needle));
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
      missing: true,
      hidden: true,
      pinned: false,
      updatedAt: "",
    };
    current.count += 1;
    current.missing &&= projectMissing(task);
    current.hidden &&= taskHiddenInCodex(task);
    current.pinned ||= Boolean(task.projectPinned);
    if (!current.path && task.cwd) current.path = task.cwd;
    if ((task.updatedAt || "") > current.updatedAt) current.updatedAt = task.updatedAt || "";
    groups.set(key, current);
  });
  const projects = [...groups.values()];
  const nameCounts = projects.reduce((counts, item) => counts.set(item.name, (counts.get(item.name) || 0) + 1), new Map());
  projects.forEach((item) => {
    item.displayName = nameCounts.get(item.name) > 1 && item.path ? `${item.name} · ${compactPath(item.path)}` : item.name;
  });
  return projects.sort((left, right) => {
    if (left.pinned !== right.pinned) return left.pinned ? -1 : 1;
    if (left.missing !== right.missing) return left.missing ? 1 : -1;
    if (left.hidden !== right.hidden) return left.hidden ? 1 : -1;
    if (right.count !== left.count) return right.count - left.count;
    return left.displayName.localeCompare(right.displayName, "zh-CN");
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
  const title = task.title || "未命名任务";
  const cwd = task.cwd || "未记录工作目录";
  const model = modelLabel(task);

  return (
    <label className={`task-row ${selected ? "is-selected" : ""}`}>
      <input type="checkbox" checked={selected} onChange={() => onToggle(task.id)} />
      <span className="check-control" aria-hidden="true">
        {selected && <Check size={13} weight="bold" />}
      </span>
      <span className="task-mark"><Archive size={20} weight="duotone" /></span>
      <span className="task-copy">
        <span className="task-title-line">
          <strong title={title}>{title}</strong>
          {task.archived && <span className="status-chip neutral" title="已归档">已归档</span>}
          {projectMissing(task) && <span className="status-chip warning" title="历史项目路径不存在；此项目未绑定本地文件夹">未绑定文件夹</span>}
          {taskHiddenInCodex(task) && <span className="status-chip warning" title="未在 Codex 侧栏显示">未在 Codex 侧栏显示</span>}
          {model && <span className="status-chip neutral" title={`原会话模型：${model}`}>历史 {model}</span>}
        </span>
        <span className="task-meta">
          <span><Clock size={13} />{formatDate(task.updatedAt)}</span>
          <span title={cwd}><Path size={13} />{cwd}</span>
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
  const [projectQuery, setProjectQuery] = useState("");
  const rootRef = useRef(null);
  const selectedProject = projects.find((item) => item.key === value);
  const filteredProjects = useMemo(
    () => projects.filter((item) => projectMatchesSearch(item, projectQuery)),
    [projectQuery, projects],
  );
  const label = selectedProject
    ? `${selectedProject.displayName || selectedProject.name} · ${selectedProject.pinned ? "置顶 · " : ""}${selectedProject.missing ? "路径不存在 · " : selectedProject.hidden ? "未在侧栏 · " : ""}${selectedProject.count}`
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
    setProjectQuery("");
  };

  return (
    <div className={`project-picker ${open ? "is-open" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="project-trigger"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-haspopup="listbox"
        title={label}
      >
        <FolderSimple size={18} weight="duotone" />
        <span title={label}>{label}</span>
        <CaretDown size={14} weight="bold" />
      </button>
      {open && (
        <div className="project-menu" role="listbox" aria-label="按项目筛选">
          <label className="project-menu-search">
            <MagnifyingGlass size={14} />
            <input
              value={projectQuery}
              onChange={(event) => setProjectQuery(event.target.value)}
              placeholder="搜索项目"
              autoFocus
            />
            {projectQuery && (
              <button type="button" onClick={() => setProjectQuery("")} aria-label="清空项目搜索" title="清空项目搜索">
                <X size={13} />
              </button>
            )}
          </label>
          <div className="project-options">
            <button
              type="button"
              className={`project-option ${value === "all" ? "is-active" : ""}`}
              onClick={() => choose("all")}
              role="option"
              aria-selected={value === "all"}
              title={`全部项目 · ${tasks.length}`}
            >
              <span>全部项目</span>
              <strong>{tasks.length}</strong>
            </button>
            {filteredProjects.map((item) => {
              const name = item.displayName || item.name;
              const status = item.pinned ? "置顶" : item.missing ? "路径不存在" : item.hidden ? "未在侧栏" : "";
              return (
                <button
                  key={item.key}
                  type="button"
                  className={`project-option ${item.missing ? "is-missing" : ""} ${value === item.key ? "is-active" : ""}`}
                  onClick={() => choose(item.key)}
                  role="option"
                  aria-selected={value === item.key}
                  title={`${name}${status ? ` · ${status}` : ""} · ${item.count}`}
                >
                  <span title={name}>{name}</span>
                  {status && <em title={status}>{status}</em>}
                  <strong>{item.count}</strong>
                </button>
              );
            })}
            {!filteredProjects.length && <div className="project-empty">没有匹配项目</div>}
          </div>
        </div>
      )}
    </div>
  );
}

function ImportProjectPicker({ projects, value, onChange }) {
  const [open, setOpen] = useState(false);
  const [projectQuery, setProjectQuery] = useState("");
  const rootRef = useRef(null);
  const selected = projects.find((item) => item.path === value);
  const filteredProjects = useMemo(
    () => projects.filter((item) => projectMatchesSearch(item, projectQuery)),
    [projectQuery, projects],
  );
  const label = selected ? (selected.displayName || selected.name) : "保持压缩包中的项目";

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
    setProjectQuery("");
  };

  return (
    <div className={`project-picker import-project-picker ${open ? "is-open" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="project-trigger"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-haspopup="listbox"
        title={label}
      >
        <FolderSimple size={18} weight="duotone" />
        <span title={label}>{label}</span>
        <CaretDown size={14} weight="bold" />
      </button>
      {open && (
        <div className="project-menu" role="listbox" aria-label="选择目标项目">
          <label className="project-menu-search">
            <MagnifyingGlass size={14} />
            <input
              value={projectQuery}
              onChange={(event) => setProjectQuery(event.target.value)}
              placeholder="搜索项目"
              autoFocus
            />
            {projectQuery && (
              <button type="button" onClick={() => setProjectQuery("")} aria-label="清空项目搜索" title="清空项目搜索">
                <X size={13} />
              </button>
            )}
          </label>
          <div className="project-options">
            <button
              type="button"
              className={`project-option ${value ? "" : "is-active"}`}
              onClick={() => choose("")}
              role="option"
              aria-selected={!value}
              title="保持压缩包中的项目"
            >
              <span>保持压缩包中的项目</span>
              <strong>默认</strong>
            </button>
            {filteredProjects.map((item) => {
              const name = item.displayName || item.name;
              return (
                <button
                  key={item.key}
                  type="button"
                  className={`project-option ${value === item.path ? "is-active" : ""}`}
                  onClick={() => choose(item.path)}
                  role="option"
                  aria-selected={value === item.path}
                  title={`${name} · ${item.count}`}
                >
                  <span title={name}>{name}</span>
                  <strong>{item.count}</strong>
                </button>
              );
            })}
            {!filteredProjects.length && <div className="project-empty">没有匹配项目</div>}
          </div>
        </div>
      )}
    </div>
  );
}

function NavButton({ active, disabled, icon: Icon, title, subtitle, onSelect }) {
  return (
    <button
      type="button"
      className={active ? "active" : ""}
      disabled={disabled}
      onClick={onSelect}
    >
      <Icon size={20} weight={active ? "fill" : "regular"} />
      <span><strong>{title}</strong><small>{subtitle}</small></span>
    </button>
  );
}

function ExportView({ environment, showToast, refreshEnvironment, onOperationChange, libraryRevision }) {
  const [tasks, setTasks] = useState([]);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("all");
  const [selected, setSelected] = useState(new Set());
  const [loading, setLoading] = useState(true);
  const [operation, setOperation] = useState("");
  const [healthReport, setHealthReport] = useState(null);
  const [healthOpen, setHealthOpen] = useState(false);
  const [healthDrawerView, setHealthDrawerView] = useState("repair");
  const [healthSelection, setHealthSelection] = useState(new Set());
  const [repairPlan, setRepairPlan] = useState(null);
  const [planning, setPlanning] = useState(false);
  const [repairReceipt, setRepairReceipt] = useState(null);
  const [repairQuery, setRepairQuery] = useState("");
  const [snapshots, setSnapshots] = useState([]);
  const [snapshotSelection, setSnapshotSelection] = useState(new Set());
  const [loadingSnapshots, setLoadingSnapshots] = useState(false);
  const [confirmSnapshotDelete, setConfirmSnapshotDelete] = useState(false);
  const [visibleLimit, setVisibleLimit] = useState(120);
  const [scanProgress, setScanProgress] = useState(null);
  const scanRunRef = useRef(null);
  const libraryRevisionRef = useRef(libraryRevision);
  const scannedTaskIdsRef = useRef(new Set());
  const healthDrawerRef = useRef(null);
  const healthTriggerRef = useRef(null);
  const codexRunning = Boolean(environment?.codexRunning);

  const load = useCallback(async (resumeToken) => {
    const continuation = typeof resumeToken === "string" ? resumeToken : undefined;
    if (scanRunRef.current && bridge.cancelBackgroundJob) bridge.cancelBackgroundJob(scanRunRef.current);
    setLoading(true);
    setScanProgress({ stage: "starting", scanned: 0, total: 0, discovered: 0 });
    if (!continuation) {
      setHealthReport(null);
      setTasks([]);
      scannedTaskIdsRef.current = new Set();
    }
    try {
      if (bridge.startTaskScan && bridge.onTaskScanProgress) {
        const runId = globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random()}`;
        scanRunRef.current = runId;
        return await new Promise((resolve, reject) => {
          let unlisten = null;
          const finish = (callback) => {
            if (scanRunRef.current === runId) scanRunRef.current = null;
            unlisten?.();
            callback();
          };
          bridge.onTaskScanProgress((event) => {
            if (event?.runId !== runId) return;
            setScanProgress(event);
            if (event.kind === "batch") {
              const additions = (event.tasks || []).filter((task) => {
                if (scannedTaskIdsRef.current.has(task.id)) return false;
                scannedTaskIdsRef.current.add(task.id);
                return true;
              });
              if (additions.length) setTasks((current) => [...current, ...additions]);
            }
            if (event.kind === "complete") {
              const completedTasks = sortTasksByUpdated(event.tasks || []);
              scannedTaskIdsRef.current = new Set(completedTasks.map((task) => task.id));
              setTasks(completedTasks);
              setHealthReport(event.health || null);
              finish(() => resolve(event));
            } else if (event.kind === "cancelled") {
              finish(() => resolve(null));
            } else if (event.kind === "timed_out") {
              showToast("error", "扫描暂停", "已保留已发现的任务，可继续扫描剩余任务或重新扫描。");
              finish(() => resolve(null));
            } else if (event.kind === "error") {
              finish(() => reject(new Error(event.message || "读取本地任务失败")));
            }
          }).then((stop) => {
            unlisten = stop;
            bridge.startTaskScan(runId, continuation).catch((error) => finish(() => reject(error)));
          }).catch(reject);
        });
      }
      const result = bridge.loadTaskLibrary
        ? await bridge.loadTaskLibrary()
        : await bridge.listTasks();
      setTasks(result.tasks || []);
      setHealthReport(result.health || null);
      return result;
    } catch (error) {
      showToast("error", "读取失败", error.message);
    } finally {
      if (!scanRunRef.current) setLoading(false);
    }
  }, [showToast]);

  useEffect(() => {
    load();
    return () => {
      if (scanRunRef.current && bridge.cancelBackgroundJob) bridge.cancelBackgroundJob(scanRunRef.current);
    };
  }, [load]);

  useEffect(() => {
    if (libraryRevisionRef.current === libraryRevision) return;
    libraryRevisionRef.current = libraryRevision;
    load();
  }, [libraryRevision, load]);

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

  useEffect(() => {
    setVisibleLimit(120);
  }, [project, query]);

  const visibleTasks = useMemo(() => filtered.slice(0, visibleLimit), [filtered, visibleLimit]);

  const selectedTasks = tasks.filter((task) => selected.has(task.id));
  const selectedBytes = selectedTasks.reduce((sum, task) => sum + (task.size || 0), 0);
  const allVisibleSelected = visibleTasks.length > 0 && visibleTasks.every((task) => selected.has(task.id));
  const repairableIds = useMemo(() => new Set(repairableTaskIds(healthReport)), [healthReport]);
  const selectedRepairableIds = useMemo(
    () => [...selected].filter((id) => repairableIds.has(id)),
    [repairableIds, selected],
  );

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
      if (allVisibleSelected) visibleTasks.forEach((task) => next.delete(task.id));
      else visibleTasks.forEach((task) => next.add(task.id));
      return next;
    });
  };

  const cancelScan = () => {
    if (scanRunRef.current && bridge.cancelBackgroundJob) {
      bridge.cancelBackgroundJob(scanRunRef.current);
      setScanProgress((current) => current ? { ...current, stage: "cancelling" } : current);
    }
  };

  const resumeScan = () => {
    if (scanProgress?.resumeToken) load(scanProgress.resumeToken);
  };

  const loadMoreOnScroll = (event) => {
    const { scrollTop, scrollHeight, clientHeight } = event.currentTarget;
    if (scrollHeight - scrollTop - clientHeight < 180) {
      setVisibleLimit((current) => Math.min(current + 120, filtered.length));
    }
  };

  const exportSelected = async () => {
    setOperation("export");
    onOperationChange("export");
    try {
      const result = await bridge.exportTasks([...selected]);
      if (!result.canceled) {
        showToast("success", `已打包 ${result.count} 个任务`, filename(result.path));
      }
    } catch (error) {
      showToast("error", "导出失败", error.message);
    } finally {
      setOperation("");
      onOperationChange("");
    }
  };

  const refreshRepairPlan = async (ids) => {
    if (!ids.length || !bridge.buildRepairPlan) {
      setRepairPlan(null);
      return;
    }
    setPlanning(true);
    try {
      setRepairPlan(await bridge.buildRepairPlan(ids));
    } catch (error) {
      showToast("error", "无法生成修复计划", error.message);
    } finally {
      setPlanning(false);
    }
  };

  const openHealth = async (preferredIds = []) => {
    healthTriggerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const candidates = repairableTaskIds(healthReport);
    const preferred = new Set(preferredIds.filter((id) => candidates.includes(id)));
    setRepairReceipt(null);
    setRepairQuery("");
    setHealthSelection(preferred);
    setHealthDrawerView("repair");
    setHealthOpen(true);
    await refreshRepairPlan(candidates);
  };

  const loadSnapshots = async () => {
    if (!bridge.listLocalSnapshots) return;
    setLoadingSnapshots(true);
    try {
      setSnapshots(await bridge.listLocalSnapshots());
    } catch (error) {
      showToast("error", "无法读取本地快照", error.message);
    } finally {
      setLoadingSnapshots(false);
    }
  };

  const openSnapshots = async () => {
    healthTriggerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setHealthDrawerView("snapshots");
    setSnapshotSelection(new Set());
    setConfirmSnapshotDelete(false);
    setHealthOpen(true);
    await loadSnapshots();
  };

  const toggleSnapshot = (path) => {
    setSnapshotSelection((current) => {
      const next = new Set(current);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });
  };

  const prepareCleanAllSnapshots = () => {
    setSnapshotSelection(new Set(snapshots.map((snapshot) => snapshot.path)));
    setConfirmSnapshotDelete(true);
  };

  const deleteSnapshots = async () => {
    if (!snapshotSelection.size || !bridge.deleteLocalSnapshots) return;
    setOperation("snapshots");
    onOperationChange("snapshots");
    try {
      const result = await bridge.deleteLocalSnapshots([...snapshotSelection]);
      setSnapshots((current) => current.filter((snapshot) => !snapshotSelection.has(snapshot.path)));
      setSnapshotSelection(new Set());
      setConfirmSnapshotDelete(false);
      showToast("success", "本地快照已删除", `已释放 ${formatBytes(result.reclaimedBytes || 0)} 空间`);
    } catch (error) {
      showToast("error", "删除快照失败", error.message);
    } finally {
      setOperation("");
      onOperationChange("");
    }
  };

  const toggleHealthTask = (id) => {
    const next = new Set(healthSelection);
    next.has(id) ? next.delete(id) : next.add(id);
    setHealthSelection(next);
  };

  const repairGroups = useMemo(
    () => buildRepairGroups(repairPlan?.items || [], tasks, repairQuery, healthSelection),
    [healthSelection, repairPlan, repairQuery, tasks],
  );

  const toggleAllRepairable = () => {
    const actionable = repairPlan?.items.filter((item) => item.canApply).map((item) => item.id) || [];
    setHealthSelection((current) => current.size === actionable.length ? new Set() : new Set(actionable));
  };

  const snapshotBytes = snapshots.reduce((sum, snapshot) => sum + (snapshot.size || 0), 0);
  const selectedSnapshotBytes = snapshots
    .filter((snapshot) => snapshotSelection.has(snapshot.path))
    .reduce((sum, snapshot) => sum + (snapshot.size || 0), 0);

  const applyHealthPlan = async () => {
    setOperation("health");
    onOperationChange("health");
    try {
      const result = await bridge.applyRepairPlan([...healthSelection]);
      const receipt = result.receipt || {};
      setRepairReceipt(receipt);
      showToast(
        "success",
        receipt.registered || receipt.titlesRepaired ? "本地任务修复完成" : "没有需要执行的修复",
        receipt.message || receipt.codexHome,
      );
      await load();
      refreshEnvironment?.();
    } catch (error) {
      showToast("error", "修复失败", error.message);
    } finally {
      setOperation("");
      onOperationChange("");
    }
  };

  useEffect(() => {
    if (!healthOpen) return undefined;
    const focusTimer = window.requestAnimationFrame(() => {
      healthDrawerRef.current?.querySelector(".health-drawer-close")?.focus();
    });
    const onKeyDown = (event) => {
      if (event.key === "Escape") {
        setHealthOpen(false);
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = [...(healthDrawerRef.current?.querySelectorAll(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ) || [])];
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(focusTimer);
      window.removeEventListener("keydown", onKeyDown);
      healthTriggerRef.current?.focus?.();
    };
  }, [healthOpen]);

  return (
    <section className={`workspace ${healthReport ? "export-workspace-has-health" : ""}`} aria-labelledby="export-title">
      <header className="workspace-header">
        <div>
          <span className="eyebrow">从这台电脑导出</span>
          <h1 id="export-title">选择要带走的任务</h1>
          <p>{loading ? `${scanProgress?.stage === "organizing" ? "正在整理任务信息" : "正在扫描"}${scanProgress?.total ? ` · ${scanProgress.scanned || 0}/${scanProgress.total}，已发现 ${scanProgress.discovered || tasks.length} 个` : "…"}` : scanProgress?.kind === "timed_out" ? `扫描已暂停 · 已发现 ${scanProgress.discovered || tasks.length}/${scanProgress.total || "?"} 个任务` : `${tasks.length} 个 Codex 任务 · 数据只在本机处理`}</p>
        </div>
        <div className="scan-actions">
          {loading && <button type="button" className="text-button compact" onClick={cancelScan}>取消扫描</button>}
          {!loading && scanProgress?.kind === "timed_out" && <button type="button" className="text-button compact" onClick={resumeScan}>继续扫描</button>}
          <button className="icon-button refresh-button" onClick={() => load()} disabled={loading} title="重新扫描" aria-label="重新扫描">
            <ArrowClockwise size={19} className={loading ? "spin" : ""} />
          </button>
        </div>
      </header>

      {!loading && healthReport?.summary?.reregisterCount > 0 && (
        <section className="health-summary" aria-label="本地任务健康摘要">
          <span className="health-summary-copy">
            <ShieldCheck size={19} weight="duotone" />
            <span><strong>{healthReport.summary.reregisterCount} 个任务暂未在 Codex 中显示</strong><small>会话仍保留在本机，可选择重新显示</small></span>
          </span>
          <button type="button" className="text-button compact health-open-button" onClick={() => openHealth()}>处理任务</button>
        </section>
      )}

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
            {allVisibleSelected ? "取消已显示任务" : `全选已显示 ${visibleTasks.length} 条`}
          </button>
          <button className="primary-button toolbar-export" disabled={!selected.size || operation} onClick={exportSelected}>
            <Export size={17} weight="bold" />
            {operation === "export" ? "正在打包…" : selected.size ? `导出 ${selected.size} 个任务` : "导出已选任务"}
          </button>
        </div>
      </div>

      <div className="task-list" aria-live="polite" onScroll={loadMoreOnScroll}>
        {loading && !tasks.length && <div className="empty-state"><ArrowClockwise size={28} className="spin" /><strong>正在扫描 Codex 任务</strong><span>扫描到的任务会立即显示在这里</span></div>}
        {!loading && !filtered.length && (
          <div className="empty-state"><MagnifyingGlass size={28} /><strong>没有找到匹配的任务</strong><span>换一个关键词试试</span></div>
        )}
        {visibleTasks.map((task) => (
          <TaskRow key={task.id} task={task} selected={selected.has(task.id)} onToggle={toggle} />
        ))}
        {filtered.length > visibleTasks.length && (
          <button className="text-button load-more-tasks" onClick={() => setVisibleLimit((current) => current + 120)}>
            显示更多任务（剩余 {filtered.length - visibleTasks.length} 个）
          </button>
        )}
      </div>

      <footer className="action-bar">
        <div>
          <strong>{selected.size ? `已选择 ${selected.size} 个任务` : "尚未选择任务"}</strong>
          <span title={selected.size ? `原始会话约 ${formatBytes(selectedBytes)}` : environment?.codexHome}>{selected.size ? `原始会话约 ${formatBytes(selectedBytes)}` : environment?.codexHome}</span>
        </div>
        <div className="action-buttons">
          <button className="primary-button" disabled={!selected.size || operation} onClick={exportSelected}>
            <Export size={18} weight="bold" />
            {operation === "export" ? "正在打包…" : "导出压缩包"}
          </button>
          <button
            className="secondary-button"
            disabled={operation
              || !healthReport
              || (!selected.size && !repairableIds.size)
              || (selected.size > 0 && !selectedRepairableIds.length)}
            onClick={() => openHealth(selectedRepairableIds)}
            title={selected.size
              ? selectedRepairableIds.length
                ? `把 ${selectedRepairableIds.length} 个可修复任务带入修复建议`
                : "当前所选任务没有可安全执行的修复项"
              : "查看本地任务健康检查与可安全执行的修复建议"}
          >
            <Wrench size={18} weight="bold" />
            {operation === "health"
              ? "正在修复…"
              : selected.size
                ? selectedRepairableIds.length ? `修复所选可修复 ${selectedRepairableIds.length} 个任务` : "所选任务无需修复"
                : repairableIds.size ? "检查并修复" : "没有需要修复的任务"}
          </button>
        </div>
      </footer>

      {healthOpen && (
        <div className="health-drawer-backdrop" role="presentation" onClick={(event) => { if (event.target === event.currentTarget) setHealthOpen(false); }}>
            <aside ref={healthDrawerRef} className="health-drawer" role="dialog" aria-modal="true" aria-label={healthDrawerView === "snapshots" ? "本地安全备份管理" : "本地任务修复建议"} tabIndex={-1}>
              <header className="health-drawer-header">
              {healthDrawerView === "snapshots" ? <span><FileArchive size={20} weight="duotone" /><span><strong>本地安全备份</strong><small>{snapshots.length} 份 · 共 {formatBytes(snapshotBytes || 0)}</small></span></span> : <span><Wrench size={20} weight="duotone" /><span><strong>修复任务</strong><small>{repairPlan?.actionableCount || 0} 个任务可安全修复</small></span></span>}
              <button className="icon-button health-drawer-close" type="button" onClick={(event) => { event.stopPropagation(); setHealthOpen(false); }} onDoubleClick={(event) => event.stopPropagation()} aria-label="关闭" title="关闭"><X size={17} /></button>
            </header>

            {healthDrawerView === "snapshots" ? (
              <>
                <p className="health-snapshot-note">修复或导入前创建的安全备份会保留在这台电脑。默认不清理；删除后无法撤销。</p>
                <div className="snapshot-list repair-plan-list">
                  {loadingSnapshots && <div className="repair-plan-empty"><ArrowClockwise size={20} className="spin" />正在读取快照…</div>}
                  {!loadingSnapshots && snapshots.map((snapshot) => (
                    <label className="repair-plan-item snapshot-item" key={snapshot.path}>
                      <input type="checkbox" checked={snapshotSelection.has(snapshot.path)} onChange={() => toggleSnapshot(snapshot.path)} />
                      <span><strong title={snapshot.name}>{snapshot.name}</strong><small>{formatDate(snapshot.modifiedAt)} · {formatBytes(snapshot.size || 0)}</small></span>
                    </label>
                  ))}
                  {!loadingSnapshots && !snapshots.length && <div className="repair-plan-empty">暂无本地安全备份</div>}
                </div>
              </>
            ) : repairReceipt ? (
              <div className="repair-receipt">
                <ShieldCheck size={20} weight="duotone" />
                <span><strong>修复已完成</strong><small>{(repairReceipt.registered || 0) + (repairReceipt.titlesRepaired || 0)} 个任务已处理；已保留本地安全备份</small></span>
              </div>
            ) : (
              <>
                <p className="health-drawer-note">选择需要处理的任务；会话内容不会被覆盖。</p>
                <div className="repair-plan-tools">
                  <label className="repair-search-field">
                    <MagnifyingGlass size={16} />
                    <input value={repairQuery} onChange={(event) => setRepairQuery(event.target.value)} placeholder="搜索项目、任务名或路径" />
                    {repairQuery && <button type="button" onClick={() => setRepairQuery("")} aria-label="清空修复搜索" title="清空搜索"><X size={14} /></button>}
                  </label>
                  <button type="button" className="text-button compact" onClick={toggleAllRepairable} disabled={!repairPlan?.actionableCount}>
                    {healthSelection.size ? "清空选择" : `全选可修复 ${repairPlan?.actionableCount || 0} 个`}
                  </button>
                </div>
                <div className="repair-plan-list">
                  {planning && <div className="repair-plan-empty"><ArrowClockwise size={20} className="spin" />正在生成修复计划…</div>}
                  {!planning && repairGroups.map((group) => (
                    <section className="repair-project-group" key={group.key}>
                      <header><span><strong>{group.name}</strong><small title={group.path}>{group.path || "未记录项目路径"}</small></span><em>{group.items.length} 个任务</em></header>
                      {group.items.map((item) => (
                        <label className={`repair-plan-item ${item.canApply ? "" : "is-blocked"}`} key={item.id}>
                          <input type="checkbox" checked={healthSelection.has(item.id)} disabled={!item.canApply} onChange={() => toggleHealthTask(item.id)} />
                          <span><strong title={item.title}>{item.title}</strong><small>{item.actions?.includes("reregister") ? "重新显示在 Codex 中" : ""}</small></span>
                        </label>
                      ))}
                    </section>
                  ))}
                  {!planning && !repairPlan?.items.length && <div className="repair-plan-empty">没有发现可安全修复的任务</div>}
                  {!planning && repairPlan?.items.length > 0 && !repairGroups.length && <div className="repair-plan-empty">没有匹配的修复建议</div>}
                </div>
              </>
            )}

            <footer className="health-drawer-footer">
              <span className="health-footer-status">{healthDrawerView === "snapshots" ? (snapshotSelection.size ? `已选择 ${snapshotSelection.size} 份 · ${formatBytes(selectedSnapshotBytes || 0)}` : "请选择要删除的备份") : repairReceipt ? "重新打开 Codex 后检查任务是否重新出现" : healthSelection.size ? `已选择 ${healthSelection.size} 个任务` : "请选择需要处理的任务"}</span>
              <div className="health-footer-actions">
                {healthDrawerView === "snapshots" ? (confirmSnapshotDelete ? <><button className="text-button compact" type="button" disabled={operation} onClick={() => setConfirmSnapshotDelete(false)}>取消</button><button className="danger-button" type="button" disabled={operation} onClick={deleteSnapshots}>{operation === "snapshots" ? "正在删除…" : `确认删除 ${snapshotSelection.size} 份`}</button></> : <><button className="text-button compact" type="button" disabled={!snapshots.length || loadingSnapshots || operation} onClick={prepareCleanAllSnapshots}>清理全部</button><button className="danger-button" type="button" disabled={!snapshotSelection.size || operation} onClick={() => setConfirmSnapshotDelete(true)}>删除所选</button></>) : !repairReceipt && <button className="primary-button" disabled={!healthSelection.size || planning || operation || codexRunning} onClick={applyHealthPlan}>{codexRunning ? "请先退出 Codex" : operation === "health" ? "正在准备安全备份…" : "安全备份并修复"}</button>}
                {healthDrawerView !== "snapshots" && repairReceipt && <button type="button" className="text-button compact" onClick={openSnapshots}>管理本地备份</button>}
              </div>
            </footer>
          </aside>
        </div>
      )}
    </section>
  );
}

function sortTasksByUpdated(tasks) {
  return [...tasks].sort((left, right) => String(right.updatedAt || "").localeCompare(String(left.updatedAt || "")));
}

function buildRepairGroups(items, tasks, query, selectedIds) {
  const taskById = new Map(tasks.map((task) => [task.id, task]));
  const needle = query.trim().toLocaleLowerCase();
  const groups = new Map();
  items.forEach((item) => {
    const task = taskById.get(item.id);
    const key = task ? projectKey(task) : item.cwd || "__unknown__";
    const name = task ? projectName(task) : (filename(item.cwd) || "未记录项目");
    const path = task?.projectPath || item.cwd || task?.cwd || "";
    const searchText = [name, path, item.title, item.cwd, item.reason, ...(item.actions || [])]
      .join(" ")
      .toLocaleLowerCase();
    if (needle && !searchText.includes(needle)) return;
    const group = groups.get(key) || { key, name, path, items: [] };
    group.items.push(item);
    groups.set(key, group);
  });
  return [...groups.values()]
    .map((group) => ({
      ...group,
      items: [...group.items].sort((left, right) => Number(selectedIds.has(right.id)) - Number(selectedIds.has(left.id))),
      hasSelected: group.items.some((item) => selectedIds.has(item.id)),
    }))
    .sort((left, right) => Number(right.hasSelected) - Number(left.hasSelected) || left.name.localeCompare(right.name, "zh-CN"));
}

function ImportView({ active, environment, showToast, onOperationChange, onLibraryChanged }) {
  const [archive, setArchive] = useState(null);
  const [restoreExisting, setRestoreExisting] = useState(false);
  const [mergeTaskIds, setMergeTaskIds] = useState(new Set());
  const [projectMappings, setProjectMappings] = useState([]);
  const [visibleTaskLimit, setVisibleTaskLimit] = useState(120);
  const [dragging, setDragging] = useState(false);
  const [working, setWorking] = useState(false);
  const [result, setResult] = useState(null);
  const codexRunning = Boolean(environment?.codexRunning);

  const acceptArchive = useCallback((next) => {
    setArchive(next);
    setResult(null);
    setRestoreExisting(false);
    setMergeTaskIds(new Set());
    setProjectMappings(prepareProjectMappings(next.projectMappings));
    setVisibleTaskLimit(120);
  }, []);

  const inspectArchivePath = useCallback(async (archivePath) => {
    try {
      const next = await bridge.inspectArchive(archivePath);
      acceptArchive(next);
    } catch (error) {
      showToast("error", "无法读取压缩包", error.message);
    }
  }, [acceptArchive, showToast]);

  const choose = async () => {
    try {
      const next = await bridge.chooseArchive();
      if (!next.canceled) acceptArchive(next);
    } catch (error) {
      showToast("error", "无法读取压缩包", error.message);
    }
  };

  useEffect(() => {
    if (!active || !bridge.onArchiveDragDrop) {
      setDragging(false);
      return undefined;
    }
    let disposed = false;
    let unlisten;
    bridge.onArchiveDragDrop((event) => {
      if (disposed) return;
      if (event.type === "enter" || event.type === "over") {
        setDragging(true);
      } else if (event.type === "leave") {
        setDragging(false);
      } else if (event.type === "drop") {
        setDragging(false);
        const [archivePath] = event.paths || [];
        if (archivePath) inspectArchivePath(archivePath);
      }
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, inspectArchivePath]);

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
      await inspectArchivePath(archivePath);
    } catch (error) {
      showToast("error", "无法读取压缩包", error.message);
    }
  };

  const runImport = async () => {
    if (!archive || result || working) return;
    setWorking(true);
    onOperationChange("import");
    try {
      const next = await bridge.importArchive(archive.path, {
        restoreExisting,
        mergeTaskIds: [...mergeTaskIds],
        projectMappings: visibleProjectMappings.map(({ sourceKey, targetCwd, keepUnbound }) => ({ sourceKey, targetCwd, keepUnbound })),
      });
      const importedCount = next.imported?.length || 0;
      const restoredCount = next.restored?.length || 0;
      const mergedCount = next.merged?.length || 0;
      const skippedCount = next.skipped?.length || 0;
      setResult(next);
      if (importedCount || restoredCount || mergedCount) onLibraryChanged?.();
      const affectedCount = importedCount + restoredCount + mergedCount;
      showToast(
        "success",
        affectedCount ? `已处理 ${affectedCount} 个任务` : "没有需要导入的任务",
        mergedCount ? `${mergedCount} 个任务已从归档补全本地记录` : skippedCount ? `${skippedCount} 个重复任务已跳过` : "重启 Codex 后即可看到",
      );
    } catch (error) {
      showToast("error", "导入失败", error.message);
    } finally {
      setWorking(false);
      onOperationChange("");
    }
  };

  const importableCount = archive?.tasks.filter((task) => !task.conflict).length || 0;
  const conflictCount = archive?.tasks.filter((task) => task.conflict).length || 0;
  const mergeableCount = archive?.tasks.filter((task) => task.conflict && task.mergePreview?.canMerge).length || 0;
  const localRicherCount = archive?.tasks.filter((task) => task.conflict && task.mergePreview?.strategy === "local_superset").length || 0;
  const divergedCount = archive?.tasks.filter((task) => task.conflict && task.mergePreview?.strategy === "diverged").length || 0;
  const actionCount = importableCount + (restoreExisting ? conflictCount : 0);
  const visibleProjectMappings = useMemo(
    () => activeProjectMappings(projectMappings, archive?.tasks || [], restoreExisting),
    [archive, projectMappings, restoreExisting],
  );
  const unresolvedMappings = visibleProjectMappings.filter((mapping) => !mapping.targetCwd && !mapping.keepUnbound).length;
  const mappingsReady = unresolvedMappings === 0;
  const visibleArchiveTasks = archive?.tasks.slice(0, visibleTaskLimit) || [];
  const updateProjectMapping = (sourceKey, targetCwd) => {
    const normalizedTarget = normalizeDisplayPath(targetCwd);
    if (isCodexWorktreePath(normalizedTarget)) {
      showToast("error", "不能绑定临时工作树", "请选择实际项目目录，不要选择 .codex\\worktrees 下的临时目录。");
      return;
    }
    setProjectMappings((current) => current.map((mapping) => (
      mapping.sourceKey === sourceKey ? { ...mapping, targetCwd: normalizedTarget, keepUnbound: false, status: normalizedTarget ? "manual" : mapping.status } : mapping
    )));
  };
  const keepProjectUnbound = (sourceKey) => {
    setProjectMappings((current) => current.map((mapping) => (
      mapping.sourceKey === sourceKey ? { ...mapping, targetCwd: "", keepUnbound: true, status: "unbound" } : mapping
    )));
  };
  const chooseMappingDirectory = async (sourceKey) => {
    try {
      const targetCwd = await bridge.chooseDirectory();
      if (targetCwd) updateProjectMapping(sourceKey, targetCwd);
    } catch (error) {
      showToast("error", "无法选择项目目录", error.message);
    }
  };
  const toggleMergeTask = (taskId) => {
    setMergeTaskIds((current) => {
      const next = new Set(current);
      next.has(taskId) ? next.delete(taskId) : next.add(taskId);
      return next;
    });
  };

  return (
    <section className="workspace import-workspace" aria-labelledby="import-title">
      <header className="workspace-header">
        <div>
          <span className="eyebrow rose">导入到这台电脑</span>
          <h1 id="import-title">恢复 Codex 任务</h1>
          <p>重复任务会先比较内容；归档更完整时可安全补全本地历史</p>
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
              <strong title={filename(archive.path)}>{filename(archive.path)}</strong>
              <small title={`${archive.tasks.length} 个任务 · 打包于 ${formatDate(archive.createdAt)}`}>{archive.tasks.length} 个任务 · 打包于 {formatDate(archive.createdAt)}</small>
            </span>
            <button className="text-button compact" onClick={choose}>重新选择</button>
          </div>

          <div className="import-list">
            {visibleArchiveTasks.map((task) => (
              <div className="import-row" key={task.id}>
                <span className={`import-state ${task.conflict ? "skip" : "ready"}`}>
                  {task.conflict ? <ArrowClockwise size={15} /> : <Check size={15} weight="bold" />}
                </span>
                <span className="task-copy">
                  <strong title={task.title || "未命名任务"}>
                    {task.title || "未命名任务"}
                    {modelLabel(task) && <span className="status-chip neutral" title={modelLabel(task)}>{modelLabel(task)}</span>}
                  </strong>
                  <span title={task.cwd || "未记录工作目录"}>{task.cwd || "未记录工作目录"}</span>
                </span>
                <span className={`status-chip ${task.conflict ? "neutral" : "ready"}`}>
                  {task.conflict ? (
                    !restoreExisting ? "已存在，将跳过"
                      : mergeTaskIds.has(task.id) ? historyDifferenceText(task.mergePreview, "archive", "将补全本地")
                        : task.mergePreview?.strategy === "archive_superset" ? historyDifferenceText(task.mergePreview, "archive")
                          : task.mergePreview?.strategy === "local_superset" ? historyDifferenceText(task.mergePreview, "local")
                            : task.mergePreview?.strategy === "identical" ? "内容一致，将恢复"
                              : task.mergePreview?.strategy === "diverged" ? "历史分叉，将保留本地"
                                : "将恢复本地历史"
                  ) : "可导入"}
                </span>
                {task.conflict && restoreExisting && task.mergePreview && (
                  task.mergePreview.canMerge ? (
                    <label className="import-merge-toggle" title={task.mergePreview.reason}>
                      <input type="checkbox" checked={mergeTaskIds.has(task.id)} disabled={Boolean(result)} onChange={() => toggleMergeTask(task.id)} />
                      <span>补全本地</span>
                    </label>
                  ) : (
                    <span className={`import-merge-note ${task.mergePreview.strategy === "diverged" ? "is-warning" : ""}`} title={task.mergePreview.reason}>
                      {task.mergePreview.strategy === "local_superset" ? "本地更完整"
                        : task.mergePreview.strategy === "identical" ? "无需补全"
                          : task.mergePreview.strategy === "diverged" ? "需人工处理"
                            : "无法比较"}
                    </span>
                  )
                )}
              </div>
            ))}
            {archive.tasks.length > visibleArchiveTasks.length && (
              <button className="text-button load-more-tasks" onClick={() => setVisibleTaskLimit((current) => current + 120)}>
                显示更多任务（剩余 {archive.tasks.length - visibleArchiveTasks.length} 个）
              </button>
            )}
          </div>

          {conflictCount > 0 && (
            <div className="setting-row choice-row">
              <span className="setting-icon"><ArrowClockwise size={19} /></span>
              <span>
                <strong>重复任务处理</strong>
                <small>
                  {conflictCount} 个任务本地历史已存在；{mergeableCount} 个可从归档安全补全
                  {localRicherCount ? `，${localRicherCount} 个本地更完整` : ""}
                  {divergedCount ? `，${divergedCount} 个历史已分叉` : ""}
                </small>
              </span>
              <span className="segmented-control" role="group" aria-label="重复任务处理">
                <button type="button" disabled={Boolean(result)} className={!restoreExisting ? "is-active" : ""} onClick={() => setRestoreExisting(false)}>跳过</button>
                <button type="button" disabled={Boolean(result)} className={restoreExisting ? "is-active" : ""} onClick={() => setRestoreExisting(true)}>从本地历史恢复</button>
              </span>
            </div>
          )}

          {codexRunning && (
            <div className="result-strip warning-strip">
              <ShieldCheck size={20} weight="duotone" />
              <span>
                <strong>请先完全退出 Codex</strong>
                <small>导入会修改本地侧栏状态；Codex 正在运行时可能把恢复结果覆盖掉</small>
              </span>
            </div>
          )}

          <section className="project-mapping-panel" aria-label="项目路径映射">
            <header className="project-mapping-header">
              <span className="setting-icon"><FolderOpen size={19} /></span>
              <span>
                <strong>确认项目路径映射</strong>
                <small>唯一同名目录已预选；多个候选目录请手动选择。没有候选时可浏览本机目录。</small>
              </span>
              <span className={`status-chip ${mappingsReady ? "ready" : "neutral"}`}>
                {mappingsReady ? "映射已确认" : `待处理 ${unresolvedMappings} 项`}
              </span>
            </header>
            <div className="project-mapping-list">
              {visibleProjectMappings.map((mapping) => (
                <div className={`project-mapping-row ${mapping.status === "ambiguous" ? "is-ambiguous" : ""}`} key={mapping.sourceKey}>
                  <span className="mapping-source">
                    <strong title={mapping.sourceName}>{mapping.sourceName}</strong>
                    <small title={mapping.sourcePath}>{mapping.sourcePath || "未记录原项目路径"} · {mapping.activeTaskCount} 个待处理任务</small>
                  </span>
                  <span className="mapping-target">
                    <span className="mapping-target-controls">
                      <select
                        value={mapping.keepUnbound ? "__keep_unbound__" : mapping.targetCwd}
                        disabled={Boolean(result)}
                        onChange={(event) => (event.target.value === "__keep_unbound__" ? keepProjectUnbound(mapping.sourceKey) : updateProjectMapping(mapping.sourceKey, event.target.value))}
                        aria-label={`选择 ${mapping.sourceName} 的本机目录`}
                      >
                        <option value="">请选择本机目录</option>
                        <option value="__keep_unbound__">保持未绑定</option>
                        {(mapping.candidates || []).map((candidate) => <option value={candidate} key={candidate}>{candidate}</option>)}
                      </select>
                      <button type="button" className="text-button compact" disabled={Boolean(result)} onClick={() => chooseMappingDirectory(mapping.sourceKey)}>浏览…</button>
                    </span>
                    <small className={mapping.targetCwd || mapping.keepUnbound ? "is-ready" : "is-warning"}>{projectMappingStatus(mapping)}</small>
                  </span>
                </div>
              ))}
              {!visibleProjectMappings.length && <div className="project-mapping-empty">本次待处理任务没有可绑定的项目路径，将保持未绑定。</div>}
            </div>
          </section>

          {result && (
            <div className="result-strip">
              <ShieldCheck size={20} weight="duotone" />
              <span><strong>导入完成</strong><small title={result.restored?.length ? "已从本地历史恢复，请重启 Codex 查看" : "请重启 Codex 刷新任务列表"}>{result.restored?.length ? "已从本地历史恢复，请重启 Codex 查看" : "请重启 Codex 刷新任务列表"}</small>{result.backups?.length > 0 && <small>已保留本地安全备份</small>}{result.receiptWarning && <small>{result.receiptWarning}</small>}</span>
              {result.receiptPath && <button className="icon-button" onClick={() => bridge.revealPath(result.receiptPath)} title="打开维护回执" aria-label="打开维护回执"><FileArchive size={18} /></button>}
              <button className="icon-button" onClick={() => bridge.revealPath(environment.codexHome)} title="打开 Codex 数据目录" aria-label="打开 Codex 数据目录"><FolderOpen size={18} /></button>
            </div>
          )}
        </div>
      )}

      <footer className="action-bar">
        <div>
          <strong>{result ? "本次导入已完成；重新选择压缩包可再次检查" : archive ? (unresolvedMappings ? `还有 ${unresolvedMappings} 个项目需要确认路径` : (restoreExisting ? `${importableCount} 个新任务，${conflictCount} 个可恢复，${mergeTaskIds.size} 个将从归档补全` : `${importableCount} 个新任务可导入`)) : "等待选择压缩包"}</strong>
          <span title={environment?.codexHome}>{environment?.codexHome}</span>
        </div>
        <button className="primary-button rose-button" disabled={!archive || !actionCount || !mappingsReady || working || codexRunning || Boolean(result)} onClick={runImport}>
          <TrayArrowDown size={18} weight="bold" />
          {working ? "正在导入…" : result ? "导入已完成" : codexRunning ? "请先退出 Codex" : unresolvedMappings ? "先确认项目映射" : "确认映射并导入"}
        </button>
      </footer>
    </section>
  );
}

export function App() {
  const [mode, setMode] = useState("export");
  const [activeOperation, setActiveOperation] = useState("");
  const [environment, setEnvironment] = useState(null);
  const [toast, setToast] = useState(null);
  const [libraryRevision, setLibraryRevision] = useState(0);
  const toastTimerRef = useRef(null);

  const refreshEnvironment = useCallback(() => {
    bridge.getEnvironment().then((next) => {
      setEnvironment((current) => {
        if (
          current
          && current.codexRunning === next.codexRunning
          && current.activeModelProvider === next.activeModelProvider
          && current.activeModel === next.activeModel
          && current.codexHome === next.codexHome
        ) {
          return current;
        }
        return next;
      });
    }).catch(() => {});
  }, []);

  useEffect(() => {
    refreshEnvironment();
    const timer = window.setInterval(refreshEnvironment, 12000);
    return () => window.clearInterval(timer);
  }, [refreshEnvironment]);

  useEffect(() => () => {
    if (toastTimerRef.current) window.clearTimeout(toastTimerRef.current);
  }, []);

  const showToast = useCallback((type, title, message) => {
    if (toastTimerRef.current) window.clearTimeout(toastTimerRef.current);
    setToast({ type, title, message });
    toastTimerRef.current = window.setTimeout(() => {
      setToast(null);
      toastTimerRef.current = null;
    }, 5000);
  }, []);

  return (
    <main className="app-shell">
      {isMacOS && <div className="window-drag-region" data-tauri-drag-region />}
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
            disabled={Boolean(activeOperation)}
            onSelect={() => setMode("export")}
          />
          <NavButton
            active={mode === "import"}
            icon={CloudArrowDown}
            title="导入任务"
            subtitle="恢复到新的电脑"
            disabled={Boolean(activeOperation)}
            onSelect={() => setMode("import")}
          />
        </nav>

        <div className="privacy-note">
          <ShieldCheck size={18} weight="duotone" />
          <span><strong>本地处理</strong><small>不会上传会话内容</small></span>
        </div>

        <div className="sidebar-footer">
          <span className="online-dot" />
          <span>
            <strong title={environment?.demo ? "浏览器演示数据" : environment?.codexRunning ? "Codex 正在运行" : "可安全导入"}>{environment?.demo ? "浏览器演示数据" : environment?.codexRunning ? "Codex 正在运行" : "可安全导入"}</strong>
            <small title={environment?.demo ? "真实测试请看桌面 app" : `当前配置：${environment?.activeModelProvider || "openai"} / ${environment?.activeModel || "未识别模型"}`}>{environment?.demo ? "真实测试请看桌面 app" : `${environment?.activeModelProvider || "openai"} / ${environment?.activeModel || "未识别模型"}`}</small>
          </span>
        </div>
      </aside>

      <div className={`view-slot ${mode === "export" ? "is-active" : "is-hidden"}`} aria-hidden={mode !== "export"}>
        <ExportView environment={environment} showToast={showToast} refreshEnvironment={refreshEnvironment} onOperationChange={setActiveOperation} libraryRevision={libraryRevision} />
      </div>
      <div className={`view-slot ${mode === "import" ? "is-active" : "is-hidden"}`} aria-hidden={mode !== "import"}>
        <ImportView active={mode === "import"} environment={environment} showToast={showToast} onOperationChange={setActiveOperation} onLibraryChanged={() => setLibraryRevision((current) => current + 1)} />
      </div>
      <Toast toast={toast} onClose={() => setToast(null)} />
    </main>
  );
}
