import { cp, mkdir, readFile, readdir } from "node:fs/promises";
import path from "node:path";

const sourceDir = path.join("src-tauri", "target", "release", "bundle", "nsis");
const files = await readdir(sourceDir);
const { version } = JSON.parse(await readFile("package.json", "utf8"));
const installer =
  files.find((file) => file.endsWith(".exe") && file.includes(version)) ??
  files.find((file) => file.endsWith(".exe"));

if (!installer) throw new Error("Tauri NSIS installer was not generated");

await mkdir("release", { recursive: true });
await cp(
  path.join(sourceDir, installer),
  path.join("release", `Codex-Session-Transfer-${version}-x64-setup.exe`),
);
