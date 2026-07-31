# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

## Product Decisions

- The app is a private local utility. Never upload Codex session contents or add analytics that transmit task data.
- Position the app as a **local Codex task continuity tool**: move, back up, inspect, and make locally preserved tasks visible again. Do not position it as a general-purpose data recovery product or promise recovery of every private Codex state.
- The core workflow has two views only: search/select/export and inspect/import.
- Health checks, recovery suggestions, conflict previews, and operation receipts belong inside those two views as panels or dialogs, never as a third top-level view.
- Keep the visual language airy and restrained: white space, low-contrast watercolor texture, cyan for export, rose for import, and compact desktop-scale typography.
- Keep the export health panel, filter toolbar, and task list on separate grid rows with visible gaps; never allow diagnostic payloads or raw session content to expand a dialog beyond its scrollable content area.
- Export archives use the `codex-session-transfer/v1` manifest and remain standard ZIP files.
- Imports are additive and idempotent. Existing task IDs are skipped; local Codex databases are backed up before registration.
- API endpoint configuration is local runtime state: restored histories must adapt to the receiving Codex configuration, while the UI distinguishes current configuration from historical task metadata.
- Read the active `model_provider` from `config.toml`; when absent, infer it from the newest local session's metadata, then rewrite restored sessions to that provider so they appear in the matching API or official sidebar.
- When a historical project folder is missing, restore it as an unbound project; never create a placeholder folder or bind it to a same-named local folder.
- Recovery is bounded: preserve additive/idempotent import, local snapshots, and explicit user review. Do not overwrite the complete Codex data directory, forcibly rewrite sidebar preferences/caches, or claim exact long-term-memory restoration.
- For sidebar visibility checks, treat a current project assignment to an existing project or the explicit projectless-thread list as authoritative; do not trust stale `thread-client-id-v1` cache entries after a project is removed.
- Recovery suggestions must be grouped and searchable by project, start with no tasks selected, and preserve an explicit selection made in the export list when opening the repair panel. Newly re-registered projects should be placed at the top of Codex’s project order rather than appended at the bottom.
- The repair panel is an action surface, not a diagnostics console: default to a short actionable count, project-grouped selectable tasks, and one safety statement. Hide database checks, technical causes, receipts, and non-actionable diagnostics by default; non-actionable items may appear only as a compact expandable count.
- Legacy malformed-title detection remains a compatibility cleanup concern, not a health or repair signal; never surface it as a recurring user action. Re-registering an archived task must create an active-session copy while retaining the archived source, so a successful repair is idempotent on the next scan.
- For Codex data compatibility, choose the highest-numbered available `state_*.sqlite` state database, scan both active and archived session trees without duplicate task rows, and use `history.jsonl` only as a fallback for a missing user-facing title.
- Task discovery must be progressive and cancellable: show parsed tasks as they are scanned, then enrich their metadata in the background. A timed-out scan retains an in-memory continuation so users can continue the remaining files without losing discovered/selected tasks; refresh intentionally starts over. Keep list rendering incremental (load more on scroll) so a large local history never freezes the window. Repair writes remain non-interruptible once a local snapshot/write begins, to avoid partial state changes.
- Export, import, and repair are mutually exclusive operations. Once one starts, keep the user in its current view and prevent a second operation from starting until it completes.
- Writes to Codex data must also be mutually exclusive across separate app processes through an atomic local lock file; a stale lock may be reclaimed only after its expiry threshold.
- Snapshot cleanup is explicit and opt-in: show local snapshot storage on demand and delete only user-selected snapshots; never auto-delete recovery snapshots.
