if (process.platform !== "win32") {
  console.error(
    "Windows installer packaging must run on Windows. Use GitHub Actions or a Windows machine to run npm run build:win.",
  );
  process.exit(1);
}
