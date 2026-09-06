# Full-window file history — 2026-09-06

## Scope and interaction

This is shared Community UI/UX maintenance, also consumed by Pro through its pinned dependency. No paid capability, data schema or version-reading behavior changes.

- The file-history dialog fills the current application window. It does not create a native window or enter macOS full-screen mode.
- The fixed header contains the existing file identity and close action. Its leading inset avoids the native window controls; the top strip preserves window dragging without imitating system controls.
- The version list is bounded to 240–320 px on desktop. The comparison/content pane takes the remaining width. Each pane retains independent vertical scrolling; the header stays outside both.
- Below 720 px, the existing stacked, independently bounded panes remain available. The desktop app still has its existing native minimum window size.
- Unified Diff retains horizontal scrolling. Split Diff wraps long lines inside each column so text cannot paint across the other version.
- Opening focuses the close button. Tab stays in the dialog, Escape closes the version menu first and then the dialog, and closing from the version badge restores focus to that badge. Background file shortcuts remain blocked while the dialog is active.
- The surface uses existing light/dark semantic colors and the existing reduced-motion preference. No other preview or settings surface is resized.

Guidance checked: [Apple HIG — Windows](https://developer.apple.com/design/human-interface-guidelines/windows), including native control placement, resizable content and avoiding custom system-window controls.

## Verification

The browser-only `?timelinePreview` fixture supplies 40 versions, a long filename, 160 long text lines per version and operation history. It does not read a real workspace or call an AI provider.

Verified in an isolated headless Chrome context:

- 1280 × 800 light, 920 × 640 light, 1600 × 1000 dark, and 640 × 720 dark with reduced motion.
- Dialog bounds equal the viewport; desktop list width stays within its limits; no document-level overflow.
- Wheel scrolling in either pane leaves the other pane and header unchanged.
- Unified horizontal scrolling, split view, version-menu Escape ordering (including before focus transfer), Tab containment, blocked background search shortcut, dialog Escape and badge focus restoration.
- Screenshots inspected after interaction at dense content, including minimum-width and dark split Diff. Long split lines remain within their columns; no page errors were observed.
- `npm test`: 159 passed. `npm run build` and `git diff --check` passed. The existing Vite large-chunk warning remains unrelated to this change.

Native window dragging/traffic-light interaction, VoiceOver, and backend-dependent loading/error/unsupported states still require desktop acceptance. Browser checks do not claim those native checks passed. No real file mutation or AI request was performed for visual verification.

## Manual acceptance

Run `npm run tauri:dev` in the desired edition. Open a file's version badge/history entry and confirm the dialog covers the entire application window; resize the window and inspect unified/split comparison. Scroll both panes separately. Open a version selector, press Escape once to close only the selector, then again to close history. Check the top drag strip and native window controls. Use synthetic files when testing version changes or error states.
