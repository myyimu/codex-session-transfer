# Codex Session Transfer

A lightweight local desktop utility for moving Codex task session history between computers.

The app scans the Codex tasks on your current machine, lets you search and filter them by project, exports selected tasks into a ZIP archive, and imports that archive into Codex on another machine. All session data stays local.

[中文 README](./README.md)

## Project Description

Codex Session Transfer is a local-first import/export tool for selected Codex task sessions. It is useful when replacing a computer, backing up important tasks, moving project context, or syncing selected Codex history across machines.

Suggested GitHub repository About description:

```text
Local-first desktop utility for exporting and importing selected Codex task sessions across computers.
```

## Features

- Scans local Codex task history and displays task titles similar to Codex.
- Searches by task title, path, and session content.
- Groups and filters tasks by project.
- Marks missing projects as deleted and places them at the bottom of the project picker.
- Exports selected tasks as a standard ZIP archive.
- Previews an archive before importing it into the current Codex data directory.
- Skips duplicate task IDs during import, so existing sessions are not overwritten.
- Creates a backup of the Codex SQLite database before importing.
- Can adapt old project paths to common local folders on the new machine.

## App Usage Guide

### Export Tasks

1. Open the app and stay on the Export Tasks page.
2. Click the refresh button to rescan local Codex tasks.
3. Search by task title, project path, or session content.
4. Use the project picker next to the search box to filter by project.
5. Select the tasks you want to move, or use Select Current Results.
6. Click an Export button and choose where to save the archive.
7. The app creates a `codex-tasks-YYYY-MM-DD.zip` archive.

### Import Tasks

1. Switch to the Import Tasks page.
2. Choose a ZIP archive created by this app.
3. Preview the tasks in the archive and review duplicate tasks.
4. Keep Adapt Project Paths enabled if you want missing old paths to be matched against `~/work`, `~/Projects`, and `~/Documents`.
5. Click Import to Codex.
6. Restart Codex to see the imported tasks.

### Duplicate and Overwrite Rules

Imports do not overwrite existing tasks. The app detects duplicates by Codex task ID:

- Existing local tasks are skipped.
- New tasks are written into the Codex session directory.
- The Codex database is backed up before importing.

## Download

Download the latest build from GitHub Releases:

- macOS: `Codex-Session-Transfer-*-aarch64.zip`
- Windows: `Codex-Session-Transfer-*-x64-setup.exe`

On macOS, unzip the archive and open the `.app`. If macOS blocks the app, right-click it in Finder and choose Open.

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
