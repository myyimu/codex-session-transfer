import { contextBridge, ipcRenderer, webUtils } from "electron";

contextBridge.exposeInMainWorld("codexBridge", {
  listTasks: () => ipcRenderer.invoke("tasks:list"),
  exportTasks: (taskIds) => ipcRenderer.invoke("tasks:export", taskIds),
  chooseArchive: () => ipcRenderer.invoke("archive:choose"),
  inspectArchive: (archivePath) => ipcRenderer.invoke("archive:inspect", archivePath),
  importArchive: (archivePath, options) =>
    ipcRenderer.invoke("archive:import", archivePath, options),
  getPathForFile: (file) => webUtils.getPathForFile(file),
  revealPath: (targetPath) => ipcRenderer.invoke("path:reveal", targetPath),
  getEnvironment: () => ipcRenderer.invoke("environment:get"),
});

