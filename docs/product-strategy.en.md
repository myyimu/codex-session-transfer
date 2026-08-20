# Codex Task Continuity: Product Strategy, UX, and Delivery Plan

Updated: 2026-08-20

[中文版本](./product-strategy.md)

## Conclusion

Recovery is valuable, but the product should not be positioned as a low-frequency, full disaster-recovery tool. A more accurate and sustainable position is: **a local-first Codex task-continuity utility that helps people move, back up, inspect, and make locally preserved tasks usable again.**

The primary users are not people who lose data once. They are heavy Codex users who work across devices, maintain several projects, and keep important task context for long periods. When a task disappears from view, they lose more than a transcript: they lose requirements, decisions, tool-execution history, and the point from which collaboration can continue.

## Research Summary

Public signals support the existence of a real local-task visibility and portability problem, although the available evidence still comes mainly from early-adopter communities and does not establish a mass-market, high-frequency need.

- OpenAI describes Codex as a separate desktop view whose history is distinct from ChatGPT history, with local tasks remaining on the computer. Local export, backup, and migration are therefore valid needs that ordinary ChatGPT synchronization should not be expected to cover. [OpenAI Help Center](https://help.openai.com/en/articles/20001275-chatgpt-work-and-codex)
- A public Codex issue reports that conversations still exist locally while the desktop app fails to discover, index, associate, or display them, and explicitly asks for a safe re-indexing or recovery mechanism. [Issue #23999](https://github.com/openai/codex/issues/23999)
- Another desktop issue describes months of history appearing missing in the GUI while remaining visible in the CLI. This makes presentation-layer mismatch a more immediate product opportunity than physical file corruption. [Issue #17354](https://github.com/openai/codex/issues/17354)
- Community tools already focus on offline packaging, preview, path mapping, and cross-device import while deliberately avoiding cloud sync and account-state migration. That validates the use case and reinforces the need for a safe, explainable local workflow. [Community example](https://www.reddit.com/r/OpenaiCodex/comments/1u569yb/unofficial_localfirst_codex_session_exportimport/)

The inference is that this may remain a niche market, but each failure carries a high context-loss and switching cost. The product does not need daily use to prove its value. It should behave like a reliable migration tool and safety device that becomes the default choice during device changes, project archiving, and sidebar visibility failures.

## Capability Value and Tradeoffs

| Capability | User value | Current status | Product decision |
| --- | --- | --- | --- |
| Selective export, import, and per-project path mapping | High: covers device changes, backups, and project handoff | Implemented | Keep as the primary workflow; explicitly confirm every local project directory before import |
| Task database, index, and project-state re-registration | High: solves “the files exist, but the sidebar does not show them” | Implemented | Keep as an explainable, user-selected repair action |
| Title and first-user-message compatibility cleanup | Medium: restores searchability and readability | Implemented as internal compatibility handling | Do not surface it as a recurring health or repair signal |
| Quit protection and local snapshots before writes | High: prevents secondary loss | Implemented | Keep as a non-bypassable safety gate |
| Pre-import manifest, path mapping, and conflict preview | High: explains impact before a write | Core workflow implemented | Continue improving per-task explanations and rejection reasons |
| Safe JSONL merge for an existing task | High but risky: preserves later local continuation | Bounded preview and append-only merge implemented | Run only after explicit selection and a successful safety check; otherwise skip |
| Archived state, internal subtask relationships, cache, and ordering repair | Medium: improves sidebar organization | Partial or not implemented | Add support gradually and only for verifiable fields |
| Long-term-memory database merge and re-summarization | Low-frequency, highly complex, high compatibility risk | Not implemented | Exclude from the core promise; retain only research and evidence-export options |
| Full data-directory replacement | Risk outweighs value | Not supported | Explicitly prohibited |

## Positioning and Information Architecture

Product positioning consistently uses “Codex Task Continuity” with the subtitle “Move, back up, inspect, and rediscover local tasks.” The current installer and window still use “Codex Session Transfer,” and the repository remains `codex-session-transfer`. A future rename should update installer metadata, window titles, READMEs, and in-app branding together.

The product always has only two top-level views:

1. **Task library**: discover, filter, export, and inspect local tasks, then repair visibility after confirmation.
2. **Import archive**: inspect a ZIP, compare it with local state, confirm local paths for each project, and perform a rollback-capable import.

Health checks, repair suggestions, conflict details, and operation receipts stay inside those views as collapsible panels or drawers rather than becoming a third top-level module.

## Layout and Interaction

### Task Library (Export)

The top area stays lightweight and shows task count and scan status. The current Codex provider, model, and running state live in the application sidebar footer. A low-contrast recovery prompt appears only when an actionable “task not shown” problem exists. Malformed titles, database consistency, and other technical counts are not exposed to ordinary users.

The body uses a compact desktop task-library layout:

- A separate toolbar contains task search, project filtering, sort context, and Select Visible Tasks.
- The main area incrementally renders task rows with title, project, last activity, conversation count, and state labels. Large histories continue loading on scroll.
- A fixed action bar shows selected count and approximate size, with Export Archive and a secondary repair action enabled only for repairable tasks.

Discovered tasks appear immediately. Scanning can be cancelled, and a timed-out scan can continue from its in-memory state. The recovery prompt opens a right-side action drawer where tasks are grouped by project, searchable, and initially unselected. The normal interface shows only the actionable count and the safety statement that a local backup will be created before writing. After completion it shows a concise result and provides access to local snapshot management; technical causes and non-actionable findings remain hidden by default.

### Import Archive

The import page follows one top-to-bottom decision flow so users do not have to move repeatedly among file, project, and conflict settings:

1. A ZIP drop zone is shown first; after parsing, the selected file, task count, and archive time are displayed.
2. The task list shows new and duplicate tasks with their default actions. Duplicate tasks can be skipped or restored from local history.
3. A per-task Merge Continuation control appears only for duplicates that pass the safe append preview, along with an append count or rejection reason.
4. The path-mapping panel lists each archived project, its original path, local candidates, and confirmation state. A unique candidate is preselected; ambiguous or missing matches require an existing local directory to be selected.
5. The fixed confirmation area summarizes new, restored, merged, or unresolved mappings, and uses the explicit action label Confirm Mapping and Import.

Duplicates are skipped by default and never overwritten. Users can choose Restore from Local History and can additionally select Merge Continuation for individual tasks that pass the safety check. The app must show whether append is safe, how many records can be appended, and why a merge was rejected. JSONL is never overwritten without a preview. When a historical project directory is missing, the user must select an existing local directory; tasks with no recorded project path remain unbound. Placeholder folders are never created.

### Visual Principles

Continue the light local-utility feel: white space, low-contrast watercolor texture, cyan for export, and rose for import. Avoid large warning-red areas; use amber for “decision required” and reserve red for conditions that actually block execution. Prefer user outcomes such as “3 tasks can reappear” over implementation details such as “write state_5.sqlite.”

## Current Delivery Status and Next Steps

### Complete: Understandable and Confirmable

- Both top-level views include progressive task scanning, visibility-recovery prompts, archive preview, and duplicate status.
- The normal UI exposes only the actionable “task not shown” suggestion; database consistency and similar technical diagnostics remain internal.
- Import and repair create local snapshots before writes and operation receipts afterward.
- Default semantics remain conservative: new tasks are imported and existing tasks are skipped; re-registration and safe merge require explicit selection.

Acceptance criterion: users can explain which tasks will change, why they will change, and how to recover without understanding the internal database.

### Complete: Bounded Safe Continuation Merge

- Existing tasks receive a JSONL append-safety preview. Merge Continuation is enabled only when ordering and shared-history constraints pass.
- Truncated records, incompatible histories, and uncertain cases are never overwritten speculatively; users can still re-register only or skip.
- Import preserves verifiable source and task metadata without constructing unknown parent-child relationships.

Acceptance criterion: messages created after a backup are not lost during recovery, and every conflict that cannot be judged safely remains in preview.

### In Progress: Maintainability and Compatibility

- Import and repair already retain rotated operation receipts and explicitly managed local snapshots. Snapshots are never deleted automatically.
- Task-library validation, database readability, and registration consistency checks remain internal diagnostics rather than normal-interface technical counts.
- Continue tracking changes in the local Codex data model and expand archived-state, source, and sidebar-visibility repair only when evidence is sufficient.
- Next priorities are real desktop-environment regression coverage, refreshed README screenshots, and continued cross-version compatibility testing.

Acceptance criterion: users can trace the reason, impact, and rollback point of an operation from its receipt without relying on memory or terminal logs.

### Not Scheduled

- Importing or recalculating long-term-memory databases, `MEMORY.md`, or summary watermarks.
- Full-directory replacement, automatic cache cleanup, or forced sidebar-order rewrites.
- Internal collaboration parent-child graph recovery that is not supported by a stable data model.

These capabilities depend on private, evolving internal state, and the cost of an incorrect recovery is higher than the value of implementing them prematurely. Any future work should begin with read-only inventory and evidence export rather than direct writes.

## Success Metrics

- Before import, users understand each task's default action and risk.
- Important tasks can be reopened and continued after a device change or sidebar visibility failure.
- Every write has a local snapshot and a human-readable operation receipt.
- The product never uploads task content and is not marketed as cloud sync, account migration, or long-term-memory recovery.
