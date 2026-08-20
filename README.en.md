# Codex Task Continuity

A local-first desktop app that keeps local Codex tasks usable through device changes, backups, cleanup, and sidebar visibility problems. It selectively exports, imports, inspects, and re-registers project task sessions.

It scans local Codex task history, displays tasks by project and title, exports selected sessions into a ZIP archive, and imports that archive into the current Codex data directory on another machine. All processing happens locally; session content is not uploaded.

[中文 README](./README.md) | [Product strategy, UX, and roadmap](./docs/product-strategy.en.md)

## Positioning

`Codex Task Continuity` solves the practical case where an important task still exists locally but is no longer easy to use: move it to another computer, keep an inspectable local archive, or re-register it when the sidebar no longer shows it.

It is not a full disaster-recovery product. It does not overwrite an entire `.codex` directory, promise every private cache or collaboration relationship, or claim exact cross-task long-term-memory recovery. Session data never leaves the local machine.

## About

Codex Session Transfer is designed for:

- Moving Codex project task sessions from an old computer to a new one.
- Syncing selected Codex task history across multiple machines without copying the entire `.codex` directory.
- Backing up important task sessions, including project paths, task titles, original session logs, and optional browser session config.
- Restoring existing local session files back into the Codex task list when the files exist locally but no longer appear in the Codex sidebar.
- Reviewing archive contents, duplicate tasks, and per-project path mappings before a local import changes Codex state.

## Features

- **Export tasks**: scan local Codex task history and export selected tasks as a ZIP archive.
- **Progressive scanning**: discovered tasks appear while scanning; scans can be cancelled and continued after a timeout.
- **Project grouping**: filter tasks by Codex project from the project picker next to the search box.
- **Task search**: search by task title, project path, and session content.
- **Project state labels**: missing projects or projects not shown in the Codex sidebar are marked and placed later in the picker.
- **Import tasks**: preview an archive and import it into the current Codex data directory.
- **Duplicate handling**: skip existing task IDs by default or re-register local history. An archive can explicitly supplement local history only when it is a strict superset; richer local history is retained and diverged histories are not auto-merged.
- **Per-project path mapping**: confirm a local folder for each project in the archive. Exact paths and a single same-name candidate are preselected; ambiguous or missing matches require user selection.
- **Write compatibility preflight**: before import or re-registration, verify the current Codex database and sidebar-state shapes; stop safely when an unknown newer shape is detected.
- **Receiving-config adaptation**: imported history keeps its original conversation and historical model metadata, while resumable session context is rewritten to the receiving computer's current Codex provider, model, and reasoning settings.
- **Path candidate discovery**: look for same-name projects among existing Codex projects and common locations such as the user directory, `~/work`, `~/Projects`, `~/Documents`, and `~/Desktop`; never create placeholder folders.
- **Running-app protection**: import is blocked while the Codex/ChatGPT desktop main process is running, preventing the client from overwriting restored sidebar state.
- **Show tasks again**: when a local session still exists but is not shown in Codex, choose the affected tasks by project and safely re-register them.
- **Local safety backups**: preserve a local Codex-state backup automatically before importing or re-registering tasks.

## Recovery Boundaries

- In scope: session portability, task titles and project metadata, making tasks visible in the sidebar again, per-project path mapping and binding, and local safety backups before writes.
- Not guaranteed: complete Codex-directory replacement, forced repair of every cache or sort preference, undeclared internal task relationships, or exact long-term-memory merging across tasks.
- Safety rule: any write to local Codex state should happen after Codex is closed, after impact has been previewed, and with a local safety backup.

See the [product strategy](./docs/product-strategy.md) for the research rationale, two-view interaction design, and delivery plan.

## Safety Backups

Before importing or making tasks visible again, the app automatically preserves a safety backup of local Codex state. Backups are never removed automatically; after a repair completes, you can open **Manage local backups** to review or remove selected backups.

- Backups stay on the current computer and are never uploaded.
- The app never clears caches automatically, changes ordering, or overwrites the entire `.codex` directory.
- After a migration or repair, reopen Codex and confirm tasks are visible and resumable before creating substantial new work.

## App Guide

### 1. Export Tasks

The app opens on the Export Tasks page. The list shows task titles, update time, project path, and conversation count. Select tasks, then click an Export button.

### 2. Filter by Project

Click the project picker next to the search box to filter tasks by project. Missing, deleted, or hidden projects are labeled so you can decide whether to migrate or restore them.

![Project picker](./docs/screenshots/readme/project-dropdown.png)

### 3. Show Tasks Again

When the Export page says that tasks are not currently shown in Codex, click **Handle tasks**. The app groups actionable tasks by project and starts with nothing selected. Choose the tasks you want, run **Back up and repair**, then reopen Codex to verify the result.

### 4. Import an Archive

Switch to Import Tasks, then drag or choose a `.zip` archive created by this app. The app previews the archive before writing anything to Codex data.

![Import archive](./docs/screenshots/readme/import-empty.png)

### 5. Handle Duplicates and Project Mapping

After selecting an archive, each task receives an import status:

- `Ready`: the task does not exist locally and can be imported.
- `Already exists, skip`: the same task ID exists locally and will not be overwritten.
- `Restore from local history`: re-register an existing local session into the Codex task list.
- `Supplement local`: available only when the archive fully contains local history and adds more records. Richer local history is retained, while diverged histories are never merged automatically.

The app shows path mappings for tasks that will actually be processed. An exact path or single same-name local directory is preselected; multiple candidates require manual selection, and a missing match can be resolved with the folder picker. Projects containing only skipped tasks do not block import. Tasks without a recorded project path remain unbound.

## Model and API Route Adaptation

During import, the app treats conversation content as portable history and provider, model, and reasoning settings as local runtime configuration. Whether a task originally used the official setup or a custom API route, its resumable session context is adapted to the receiving computer's current Codex configuration.

- The exported ZIP keeps the original session content and task history.
- Historical provider and model labels shown in the task list identify the source only; they do not mean the imported task will keep using that configuration.
- During import or restore, resumable session context is rewritten to the receiving computer's current provider, model, and reasoning settings.
- The app does not migrate third-party API keys, route settings, proxy settings, or external model service accounts.

If you want the new computer to use the same third-party route as the old one, configure that provider, its API key, and network settings on the new computer before importing.

## Usage

### Export from the old computer

1. Open `Codex Session Transfer`.
2. Go to Export Tasks.
3. Use search or the project picker to find the sessions you want to move.
4. Select tasks, or use Select Current Results.
5. Click Export Archive and save the generated `codex-tasks-YYYY-MM-DD.zip`.

### Import on the new computer

1. Copy the ZIP archive to the new computer.
2. Fully quit the Codex/ChatGPT desktop app: use `Cmd + Q` on macOS; on Windows, quit from the tray or app menu and confirm that the main process has stopped.
3. Open `Codex Session Transfer` and go to Import Tasks.
4. Choose the ZIP archive and review the preview.
5. Review duplicate handling and confirm the local path mapping for each project.
6. Click Confirm Mapping and Import.
7. Reopen Codex/ChatGPT and check the restored projects and tasks in the sidebar.

## Duplicate and Restore Rules

Imports do not overwrite existing tasks by default. The app detects duplicates by Codex task ID:

- New task IDs are written into the Codex session directory.
- Existing local tasks are skipped by default.
- Restore from local history re-registers an existing local session into the Codex task list and clears archived/hidden state.
- Duplicate histories are compared as ordered session records after receiving-machine path, provider, and model fields are normalized. When the archive is a strict superset, you can explicitly choose **Supplement local**.
- A local strict superset is retained, identical histories are left unchanged, and histories with additions on both sides are marked as diverged. The app never guesses from timestamps or overwrites JSONL speculatively.
- Project mappings accept existing local directories only. A missing historical project is never replaced with a newly created placeholder folder.
- Imported archive sessions are adapted to the receiving computer's current Codex provider, model, and reasoning settings.
- Import is blocked while Codex/ChatGPT is running to avoid sidebar state being overwritten by the running client.

## Download

Download the latest build from GitHub Releases:

**Latest releases:** [github.com/myyimu/codex-session-transfer/releases/latest](https://github.com/myyimu/codex-session-transfer/releases/latest)

- macOS: `Codex-Session-Transfer-*-arm64.zip`
- Windows: `Codex-Session-Transfer-*-x64-setup.exe`

On macOS, unzip the archive and open the `.app`. If macOS blocks the app, right-click it in Finder and choose Open.

### macOS Permission

If macOS says the app cannot be opened because the developer cannot be verified, try this first:

1. Right-click `Codex Session Transfer.app` or `Codex 会话迁移.app` in Finder.
2. Choose Open.
3. Confirm Open in the system dialog.

If it is still blocked and you trust the release downloaded from this repository, remove the quarantine attribute:

```bash
xattr -dr com.apple.quarantine "/Applications/Codex 会话迁移.app"
open "/Applications/Codex 会话迁移.app"
```

If the app is still in Downloads, replace the path with its actual location:

```bash
xattr -dr com.apple.quarantine "$HOME/Downloads/Codex 会话迁移.app"
open "$HOME/Downloads/Codex 会话迁移.app"
```

## Local Development

```bash
npm install
npm run dev
```

Web-only preview:

```bash
npm run dev:web
```

## Packaging

macOS:

```bash
npm run build:mac
```

Windows:

```bash
npm run build:win
```

Build outputs are written to `release/`.

## Archive Format

The exported ZIP contains a root `manifest.json`. Each task is stored under `tasks/<task-id>/` and includes the original `session.jsonl` plus an optional `browser.toml`.

An archive may contain complete conversations, project paths, tool-execution context, and browser session configuration. Treat the ZIP as sensitive local data; do not upload it to public file shares, issues, or chat rooms.

Current schema version:

```text
codex-session-transfer/v1
```

## Privacy

The app only reads your local Codex data directory and creates or imports local ZIP files. It does not upload session content and does not require a server. You are responsible for exported archives; uninstalling the app does not remove ZIP files saved elsewhere.

## License

[MIT](./LICENSE)

## Star

If this little tool helped you move your Codex sessions safely, a Star would make it very happy and keep the maintainer smiling for quite a while.
