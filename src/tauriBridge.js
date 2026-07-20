import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

function canceled() {
  return { canceled: true };
}

export const tauriBridge = {
  listTasks: () => invoke("list_tasks"),
  async exportTasks(taskIds) {
    const destination = await save({
      title: "导出 Codex 任务",
      defaultPath: `codex-tasks-${new Date().toISOString().slice(0, 10)}.zip`,
      filters: [{ name: "ZIP 压缩包", extensions: ["zip"] }],
    });
    return destination ? invoke("export_tasks", { taskIds, destination }) : canceled();
  },
  restoreLocalTasks: (taskIds) => invoke("restore_local_tasks", { taskIds }),
  async chooseArchive() {
    const archivePath = await open({
      title: "选择 Codex 任务压缩包",
      multiple: false,
      directory: false,
      filters: [{ name: "Codex 任务压缩包", extensions: ["zip"] }],
    });
    return archivePath ? invoke("inspect_archive", { archivePath }) : canceled();
  },
  inspectArchive: (archivePath) => invoke("inspect_archive", { archivePath }),
  importArchive: (archivePath, options) => invoke("import_archive", { archivePath, options }),
  getPathForFile: () => null,
  async revealPath(targetPath) {
    await revealItemInDir(targetPath);
    return { ok: true };
  },
  getEnvironment: () => invoke("get_environment"),
};
