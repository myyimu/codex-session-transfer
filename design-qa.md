# Design QA

> Historical evidence notice (2026-08-20): this file records earlier visual QA runs. The absolute source and implementation paths below are retained for provenance and may no longer exist. Because the import flow now includes per-project path mapping and safe continuation merge controls, these captures are not current release-acceptance evidence.

- Source visual truth: `/var/folders/_q/vhx98_y91hlgsx7pml4js18r0000gn/T/codex-clipboard-6b8db959-d0f9-4264-8340-e4de81c05cdf.jpg`
- Export implementation: `/Users/yimu/work/codex-session-transfer/design-export.png`
- Import implementation: `/Users/yimu/work/codex-session-transfer/design-import-final.png`
- Combined comparison: `/Users/yimu/work/codex-session-transfer/design-comparison.png`
- Viewport: 1120 x 760
- State: populated export list; populated import preview with one duplicate task

## Full-view comparison evidence

The reference is a mobile visual-style reference rather than a layout specification. The implementation intentionally keeps its watercolor blue/pink balance, translucent white surfaces, generous negative space, soft hierarchy, and compact secondary text while translating the composition into a desktop utility. The combined comparison confirms that the app carries the same lightness without copying the reference application's controls or phone layout.

## Focused region evidence

The 1120 x 760 captures keep list titles, timestamps, paths, statuses, and persistent actions readable at native scale, so separate crops were not needed. Export row density and the import archive/settings/action regions were inspected directly in the full-resolution captures.

## Findings

- No remaining P0, P1, or P2 issues.
- Typography: system UI fonts use clear 27 px page headings, compact 13 px task titles, neutral line height, zero letter spacing, and ellipsis for long titles and paths.
- Spacing and layout: the 252 px sidebar, 42 px workspace inset, 7-8 px radii, stable list rows, and bottom action bar remain aligned without overflow at the target viewport.
- Colors and tokens: cyan marks export/navigation, rose marks import, green marks safe/ready states, and the off-white watercolor bitmap preserves readable contrast without a one-note palette.
- Image quality: the generated 1536 x 1024 watercolor bitmap remains sharp at the target viewport and contains no embedded UI, text, logos, or placeholder shapes.
- Copy: controls use short action labels and task names mirror Codex data. Privacy, conflict, backup, and restart outcomes are explicit.
- Interaction states: search, selection, enabled/disabled actions, archive preview, duplicate skip, path adaptation, import completion, hover/focus, loading, empty, and toast states are implemented.

## Comparison history

1. Initial import capture found a P1 layout issue: with only three top-level rows, the shared four-row workspace grid stretched the bottom action bar into a large empty panel.
2. Fixed by giving the import workspace a dedicated `auto / minmax(0, 1fr) / auto` grid.
3. Post-fix evidence in `design-import-final.png` shows the settings and action bar at stable compact heights with the watercolor negative space left intentional.

## Follow-up polish

- P3: a public macOS release should be code-signed and notarized so Gatekeeper can identify the publisher.

## Verification

- Primary interactions tested: search, select, export completion, import navigation, archive preview, duplicate status, import completion.
- Console errors checked: none.
- final result: passed

---

# 2026-07-29 修复建议抽屉排版复核

- Source visual truth: `C:/Users/Legion/AppData/Local/Temp/codex-clipboard-74827dd1-43e7-455f-bcd4-ac81a239228e.png`
- Implementation evidence: browser-rendered local Vite preview at `http://127.0.0.1:4173/` (1280 × 720 CSS px); the browser demo data does not expose a repair-plan drawer state.
- State: populated export list after the drawer-footer layout change.

## Findings

- [Resolved P1] Persistent drawer controls could overflow the narrow footer and force the status copy into character-by-character wrapping.
  - Fix: the footer now uses a two-column grid, with status copy allowed to shrink and the three actions grouped in `.health-footer-actions`; the drawer has a 540 px desktop maximum and a wrapping mobile action layout.
  - Evidence: `npm run build:web` succeeds; the running 1280 × 720 preview keeps the main list, toolbar and persistent export action bar within the viewport.

## Required fidelity surfaces

- Typography: preserves the existing compact 10–12 px utility copy and 12 px action labels; status copy has a dedicated shrinking column rather than a forced narrow flex item.
- Spacing and layout rhythm: footer padding and 8 px button gaps remain consistent; the actions are contained as one aligned group.
- Colors and tokens: no color changes; cyan action semantics, translucent surfaces and watercolor background are retained.
- Image quality and asset fidelity: no imagery or assets were changed.
- Copy and content: repair, verification and receipt labels are unchanged.

## Test gap

The browser-only demo bridge does not return a health report, so it cannot open the native repair-plan drawer for a screenshot matched to the supplied drawer reference. The code path is compiled successfully, but this exact native state still needs one visual pass in the desktop app.

- historical result on 2026-07-29: blocked for an exact native repair-drawer capture; this is not an assessment of the current build
