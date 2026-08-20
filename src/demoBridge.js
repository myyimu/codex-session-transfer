const now = Date.now();

const demoTasks = [
  ["019f7091", "当前项目远程有一个 pr 判断是否可以或者需要合并", "/Users/yimu/work/ai-novel-diagnosis", 18, 0],
  ["019f5acb", "规划并落地小说质检指标与编辑改稿产品方向", "/Users/yimu/work/ai-novel-diagnosis", 42, 1],
  ["019f487c", "完善第五项并提交", "/Users/yimu/work/ai-novel-diagnosis", 27, 2],
  ["019f392a", "诊断本地 LLM 接口连接限制", "/Users/yimu/work/ai-novel-diagnosis", 14, 4],
  ["019f214d", "检查章节 Map 原子事实扩展契约", "/Users/yimu/work/ai-novel-diagnosis", 9, 8],
  ["019f106e", "小说预览页面视觉打磨", "/Users/yimu/work/ai_novel_book_preview_v18_visual_polish", 31, 12],
].map(([id, title, cwd, turns, days], index) => ({
  id: `${id}-demo-${index}`,
  title,
  cwd,
  projectKey: index !== 5 ? cwd : `${cwd}/missing-preview-project`,
  projectName: index !== 5 ? cwd.split(/[\\/]/).pop() : "missing-preview-project",
  projectPath: index !== 5 ? cwd : `${cwd}/missing-preview-project`,
  projectExists: index !== 5,
  preview: title,
  updatedAt: new Date(now - days * 86400000).toISOString(),
  createdAt: new Date(now - (days + 2) * 86400000).toISOString(),
  userMessageCount: turns,
  messageCount: turns * 2,
  size: (index + 2) * 720000,
  archived: index === 4,
  gitBranch: index < 5 ? "master" : "dev",
}));

let archive = null;
let snapshots = [
  { path: "/Users/yimu/.codex/state_5.sqlite.backup-demo", name: "state_5.sqlite.backup-demo", size: 1860000, modifiedAt: new Date(now - 7200000).toISOString() },
  { path: "/Users/yimu/.codex/sqlite/codex-dev.db.backup-demo", name: "codex-dev.db.backup-demo", size: 920000, modifiedAt: new Date(now - 86400000).toISOString() },
];

export const demoBridge = {
  async listTasks() {
    await new Promise((resolve) => setTimeout(resolve, 320));
    return { tasks: demoTasks, codexHome: "/Users/yimu/.codex" };
  },
  async exportTasks(taskIds) {
    await new Promise((resolve) => setTimeout(resolve, 650));
    return { canceled: false, count: taskIds.length, path: "/Users/yimu/Downloads/codex-tasks-demo.zip", size: 4200000 };
  },
  async restoreLocalTasks(taskIds) {
    await new Promise((resolve) => setTimeout(resolve, 520));
    return {
      restored: demoTasks.filter((task) => taskIds.includes(task.id)),
      backups: ["state_5.sqlite.backup-demo"],
      codexHome: "/Users/yimu/.codex",
    };
  },
  async listLocalSnapshots() {
    await new Promise((resolve) => setTimeout(resolve, 180));
    return snapshots;
  },
  async deleteLocalSnapshots(snapshotPaths) {
    await new Promise((resolve) => setTimeout(resolve, 260));
    const selected = new Set(snapshotPaths);
    const deleted = snapshots.filter((snapshot) => selected.has(snapshot.path));
    snapshots = snapshots.filter((snapshot) => !selected.has(snapshot.path));
    return { deletedCount: deleted.length, reclaimedBytes: deleted.reduce((sum, snapshot) => sum + snapshot.size, 0) };
  },
  async chooseArchive() {
    await new Promise((resolve) => setTimeout(resolve, 280));
    archive = {
      canceled: false,
      path: "/Users/yimu/Downloads/codex-tasks-2026-07-19.zip",
      createdAt: new Date(now - 3600000).toISOString(),
      tasks: demoTasks.slice(0, 4).map((task, index) => ({
        ...task,
        conflict: index === 2,
        mergePreview: index === 2 ? {
          canMerge: true,
          strategy: "archive_superset",
          archiveRecordCount: 68,
          localRecordCount: 61,
          appendRecordCount: 7,
          archiveUniqueRecordCount: 7,
          localUniqueRecordCount: 0,
          archiveUniqueUserTurnCount: 2,
          localUniqueUserTurnCount: 0,
          resultRecordCount: 68,
          reason: "归档完整包含本机会话，可安全补全本地缺少的记录。",
        } : null,
      })),
      projectMappings: [
        {
          sourceKey: demoTasks[0].projectKey,
          sourceName: demoTasks[0].projectName,
          sourcePath: demoTasks[0].cwd,
          candidates: [demoTasks[0].cwd],
          suggestedPath: demoTasks[0].cwd,
          status: "exact",
          taskCount: 4,
          taskIds: demoTasks.slice(0, 4).map((task) => task.id),
        },
      ],
    };
    return archive;
  },
  async inspectArchive() {
    return this.chooseArchive();
  },
  async chooseDirectory() {
    await new Promise((resolve) => setTimeout(resolve, 180));
    return "/Users/yimu/work/migrated-project";
  },
  async importArchive(_archivePath, options = {}) {
    await new Promise((resolve) => setTimeout(resolve, 700));
    const mappings = new Map((options.projectMappings || []).map((mapping) => [mapping.sourceKey, mapping]));
    const mappedCwd = (task) => {
      const mapping = mappings.get(task.projectKey);
      if (!mapping) return task.cwd;
      return mapping.keepUnbound ? "" : mapping.targetCwd;
    };
    const imported = archive.tasks
      .filter((task) => !task.conflict)
      .map((task) => ({ ...task, cwd: mappedCwd(task) }));
    const restored = options.restoreExisting
      ? archive.tasks.filter((task) => task.conflict).map((task) => ({ ...task, cwd: mappedCwd(task) }))
      : [];
    const merged = archive.tasks
      .filter((task) => options.mergeTaskIds?.includes(task.id))
      .map((task) => ({ id: task.id, title: task.title, strategy: "archive_superset", archiveAddedRecords: task.mergePreview?.archiveUniqueRecordCount || 0 }));
    return {
      imported,
      restored,
      merged,
      skipped: options.restoreExisting ? [] : archive.tasks.filter((task) => task.conflict),
      backups: ["state_5.sqlite.backup-demo"],
      codexHome: "/Users/yimu/.codex",
    };
  },
  getPathForFile(file) { return file.name; },
  async revealPath() { return { ok: true }; },
  async getEnvironment() { return { codexHome: "/Users/yimu/.codex", platform: "darwin", version: "0.1.0", demo: true, codexRunning: false, codexProcesses: [], activeModelProvider: "openai", activeModel: "gpt-5.5", activeReasoningEffort: "high" }; },
};
