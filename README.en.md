# Codex Task Continuity

A local-first desktop app that keeps local Codex tasks usable through device changes, backups, cleanup, and sidebar visibility problems. It selectively exports, imports, inspects, and re-registers project task sessions.

It scans local Codex task history, displays tasks by project and title, exports selected sessions into a ZIP archive, and imports that archive into the current Codex data directory on another machine. All processing happens locally; session content is not uploaded.

[中文 README](./README.md) | [Product strategy, UX, and roadmap](./docs/product-strategy.md)

## Positioning

`Codex Task Continuity` solves the practical case where an important task still exists locally but is no longer easy to use: move it to another computer, keep an inspectable local archive, or re-register it when the sidebar no longer shows it.

It is not a full disaster-recovery product. It does not overwrite an entire `.codex` directory, promise every private cache or collaboration relationship, or claim exact cross-task long-term-memory recovery. Session data never leaves the local machine.

## About

Codex Session Transfer is designed for:

- Moving Codex project task sessions from an old computer to a new one.
- Syncing selected Codex task history across multiple machines without copying the entire `.codex` directory.
- Backing up important task sessions, including project paths, task titles, original session logs, and optional browser session config.
- Restoring existing local session files back into the Codex task list when the files exist locally but no longer appear in the Codex sidebar.
- Reviewing archive contents, duplicate tasks, and target projects before a local import changes Codex state.

## Features

- **Export tasks**: scan local Codex task history and export selected tasks as a ZIP archive.
- **Progressive scanning**: discovered tasks appear while scanning; scans can be cancelled and continued after a timeout.
- **Project grouping**: filter tasks by Codex project from the project picker next to the search box.
- **Task search**: search by task title, project path, and session content.
- **Project state labels**: missing projects or projects not shown in the Codex sidebar are marked and placed later in the picker.
- **Import tasks**: preview an archive and import it into the current Codex data directory.
- **Duplicate handling**: skip existing task IDs by default, or restore existing local history back into the Codex task list.
- **Target project selection**: choose which local project imported or restored tasks should appear under.
- **Third-party routed session adaptation**: sessions created through `cc switch` or another third-party API route are imported as normal Codex sessions and continue with the new computer's current Codex default model.
- **Path adaptation**: map old missing project paths to same-name folders under `~/work`, `~/Projects`, and `~/Documents`.
- **Running-app protection**: import is blocked while the Codex/ChatGPT desktop main process is running, preventing the client from overwriting restored sidebar state.
- **Show tasks again**: when a local session still exists but is not shown in Codex, choose the affected tasks by project and safely re-register them.
- **Local safety backups**: preserve a local Codex-state backup automatically before importing or re-registering tasks.

## Recovery Boundaries

- In scope: session portability, task titles and project metadata, making tasks visible in the sidebar again, target-project binding, and local safety backups before writes.
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

### 5. Handle Duplicates and Target Project

After selecting an archive, each task receives an import status:

- `Ready`: the task does not exist locally and can be imported.
- `Already exists, skip`: the same task ID exists locally and will not be overwritten.
- `Restore from local history`: re-register an existing local session into the Codex task list.

You can also choose a target project so imported or restored tasks appear under the intended Codex project group.

![Import settings](./docs/screenshots/readme/import-settings.png)

## Third-Party API Routed Sessions

If some tasks on the old computer were created through `cc switch` or another third-party API route, the app migrates them as Codex history sessions instead of migrating the third-party service configuration.

- The exported ZIP keeps the original session content and task history.
- During import or restore, the task is registered as a normal local Codex session on the new computer.
- When you continue the task in Codex, it uses the new computer's current Codex default model instead of the old third-party API provider.
- The app does not migrate third-party API keys, route settings, proxy settings, or external model service accounts.

This is intended for bringing history back into Codex and continuing from there. If you want the new computer to keep using the same third-party route, configure that route separately on the new computer.

## Usage

### Export from the old computer

1. Open `Codex Session Transfer`.
2. Go to Export Tasks.
3. Use search or the project picker to find the sessions you want to move.
4. Select tasks, or use Select Current Results.
5. Click Export Archive and save the generated `codex-tasks-YYYY-MM-DD.zip`.

### Import on the new computer

1. Copy the ZIP archive to the new computer.
2. Fully quit the Codex/ChatGPT desktop app, preferably with `Cmd + Q` on macOS.
3. Open `Codex Session Transfer` and go to Import Tasks.
4. Choose the ZIP archive and review the preview.
5. Choose duplicate handling, target project, and path adaptation options.
6. Click Import to Codex.
7. Reopen Codex/ChatGPT and check the restored projects and tasks in the sidebar.

## Duplicate and Restore Rules

Imports do not overwrite existing tasks by default. The app detects duplicates by Codex task ID:

- New task IDs are written into the Codex session directory.
- Existing local tasks are skipped by default.
- Restore from local history re-registers an existing local session into the Codex task list and clears archived/hidden state.
- Target project selection binds imported or restored tasks to the selected local project.
- Import is blocked while Codex/ChatGPT is running to avoid sidebar state being overwritten by the running client.

## Download

Download the latest build from GitHub Releases:

**Latest releases:** [github.com/myyimu/codex-session-transfer/releases/latest](https://github.com/myyimu/codex-session-transfer/releases/latest)

- macOS: `Codex-Session-Transfer-*-arm64.zip`
- Windows: `Codex-Session-Transfer-*-Windows-portable.zip`

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

Current schema version:

```text
codex-session-transfer/v1
```

## Privacy

The app only reads your local Codex data directory and creates or imports local ZIP files. It does not upload session content and does not require a server.

## License

[MIT](./LICENSE)

## Star

If this little tool helped you move your Codex sessions safely, a Star would make it very happy and keep the maintainer smiling for quite a while.
