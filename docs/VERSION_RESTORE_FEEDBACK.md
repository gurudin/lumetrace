# Version restore feedback

2026-09-11. No navigation, storage or restore semantics changed.

The old-version action appears on row hover or keyboard focus, stays visible
while restoring, and remains available without hover. Reserved space avoids
layout/pointer jumps. Immediate spinner and localized restoring text use no
invented percentage. A synchronous guard blocks repeated clicks before IPC;
failure restores retry. Reduced motion keeps the text without spinning.

Optional authorName is shared by the full timeline, inspector and document/image
version rails. Absent names keep the user-edit fallback; task labels are unchanged.

Apple's current Progress Indicators guidance was checked. Real shared UI with
isolated synthetic IPC passed 24-row/long-note/421-file acceptance at 1280×800
light and 920×800 dark: hover, keyboard focus, off-hover busy visibility, animation,
one request for repeated clicks, delayed failure/retry and success. Rendered
screenshots were inspected. Community's 197 tests and production build passed.
Native WebView and actual remote network latency remain manual acceptance.
