import { cp, mkdir, readdir } from "node:fs/promises";
import path from "node:path";

const sourceDir = path.join("src-tauri", "target", "release", "bundle", "nsis");
const files = await readdir(sourceDir);
const installer = files.find((file) => file.endsWith(".exe"));

if (!installer) throw new Error("Tauri NSIS installer was not generated");

await mkdir("release", { recursive: true });
await cp(path.join(sourceDir, installer), path.join("release", "Codex-Session-Transfer-0.1.6-x64-setup.exe"));
