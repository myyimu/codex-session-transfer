import { cp, mkdir, readFile, readdir } from "node:fs/promises";
import path from "node:path";

const sourceDir = path.join("src-tauri", "target", "release", "bundle", "nsis");
let files;
try {
  files = await readdir(sourceDir);
} catch (error) {
  if (error?.code === "ENOENT") {
    throw new Error(
      `Tauri NSIS output was not found at ${sourceDir}. Windows installers are generated only on Windows.`,
    );
  }
  throw error;
}
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
