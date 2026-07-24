# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

## Product Decisions

- The app is a private local utility. Never upload Codex session contents or add analytics that transmit task data.
- The core workflow has two views only: search/select/export and inspect/import.
- Keep the visual language airy and restrained: white space, low-contrast watercolor texture, cyan for export, rose for import, and compact desktop-scale typography.
- Export archives use the `codex-session-transfer/v1` manifest and remain standard ZIP files.
- Imports are additive and idempotent. Existing task IDs are skipped; local Codex databases are backed up before registration.
- API endpoint configuration is local runtime state: restored histories must adapt to the receiving Codex configuration, while the UI distinguishes current configuration from historical task metadata.
- Read the active `model_provider` from `config.toml`; when absent, infer it from the newest local session's metadata, then rewrite restored sessions to that provider so they appear in the matching API or official sidebar.
- When a historical project folder is missing, restore it as an unbound project; never create a placeholder folder or bind it to a same-named local folder.
