import { app, BrowserWindow, dialog, ipcMain, shell } from "electron";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  exportTaskArchive,
  getCodexHome,
  importTaskArchive,
  inspectTaskArchive,
  listCodexTasks,
} from "./codex-store.mjs";

const currentDir = path.dirname(fileURLToPath(import.meta.url));

function createWindow() {
  const window = new BrowserWindow({
    width: 1120,
    height: 760,
    minWidth: 900,
    minHeight: 620,
    show: false,
    title: "Codex 会话迁移",
    titleBarStyle: process.platform === "darwin" ? "hiddenInset" : "default",
    trafficLightPosition: { x: 18, y: 18 },
    backgroundColor: "#f8fbfb",
    webPreferences: {
      preload: path.join(currentDir, "preload.mjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  window.once("ready-to-show", () => window.show());

  if (process.env.VITE_DEV_SERVER_URL) {
    window.loadURL(process.env.VITE_DEV_SERVER_URL);
  } else {
    window.loadFile(path.join(currentDir, "..", "dist", "index.html"));
  }
}

ipcMain.handle("tasks:list", () => listCodexTasks());

ipcMain.handle("tasks:export", async (_event, taskIds) => {
  const defaultName = `codex-tasks-${new Date().toISOString().slice(0, 10)}.zip`;
  const result = await dialog.showSaveDialog({
    title: "导出 Codex 任务",
    defaultPath: defaultName,
    filters: [{ name: "ZIP 压缩包", extensions: ["zip"] }],
  });
  if (result.canceled || !result.filePath) return { canceled: true };
  return exportTaskArchive(taskIds, result.filePath);
});

ipcMain.handle("archive:choose", async () => {
  const result = await dialog.showOpenDialog({
    title: "选择 Codex 任务压缩包",
    properties: ["openFile"],
    filters: [{ name: "Codex 任务压缩包", extensions: ["zip"] }],
  });
  if (result.canceled || !result.filePaths[0]) return { canceled: true };
  return inspectTaskArchive(result.filePaths[0]);
});

ipcMain.handle("archive:inspect", (_event, archivePath) =>
  inspectTaskArchive(archivePath),
);

ipcMain.handle("archive:import", (_event, archivePath, options) =>
  importTaskArchive(archivePath, options),
);

ipcMain.handle("path:reveal", async (_event, targetPath) => {
  shell.showItemInFolder(targetPath);
  return { ok: true };
});

ipcMain.handle("environment:get", () => ({
  codexHome: getCodexHome(),
  platform: process.platform,
  version: app.getVersion(),
}));

app.whenReady().then(() => {
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

