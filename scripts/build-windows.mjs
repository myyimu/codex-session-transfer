import { spawn } from "node:child_process";

if (process.platform !== "win32") {
  console.error(
    "Windows installer packaging must run on Windows. Use GitHub Actions or a Windows machine to run npm run build:win.",
  );
  process.exit(1);
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: "inherit", shell: false });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`${command} ${args.join(" ")} exited with ${code}`));
      }
    });
  });
}

await run("tauri.cmd", ["build", "--bundles", "nsis"]);
await import("./package-windows.mjs");
