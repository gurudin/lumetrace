# Version notes and milestone stars

These are shared **Community and Pro** capabilities, explicitly confirmed by the user. They are not gated paid features. Pro consumes the same implementation through its pinned Community dependency; the editions still keep separate application data.

## Interaction

- Every version row in Markdown, plain-text, image, PDF and full-window history has a star and a note action. Selecting a version, viewing Diff and restoring a version keep their existing behavior.
- A yellow star marks that specific version as a milestone, with light/dark contrast independent of the app accent. It can be removed independently of the note. Starring never changes the current-version pointer or chronological ordering.
- Notes are shown below the version details, with a two-line preview and full text available in the editor. Empty notes show “Add note”. This is a lightweight milestone annotation, not a Git tag namespace or multi-tag system.
- The note editor is a short, centered modal task with Save/Cancel, a **50-character** save limit, visible focus and native input context menus. Frontend and backend both count Unicode scalars (a supplementary character is not counted as two UTF-16 units). Over-limit drafts show a warning and disable saving rather than silently truncating pasted text. Existing longer notes remain intact; shortening is required only when saving a changed note, not when changing its star. Escape cancels only the note editor; Command/Ctrl+Enter or Command/Ctrl+S saves. IME composition does not trigger these shortcuts. Clicking outside does not discard the draft.
- Pending writes disable duplicate operations. A failed write preserves the draft and keeps the editor open for retry. Focus returns to the invoking note control after the dialog is removed and the control re-enabled.
- Version selection and metadata buttons are siblings, not nested interactive elements. Current selection, scroll ownership and dark image-preview contrast are preserved. Eight supported languages have matching copy.
- Hovering a grid/card version-count badge for 350 ms opens a nonmodal summary of the **five most recent versions**, newest first, with version number, timestamp, current-version label, yellow milestone star and note. This does not select the file, read its contents or reorder versions. Older milestones remain available in the full timeline; they do not displace more recent versions in this summary.
- The summary stays open while the pointer moves into it, owns its bounded vertical scrolling and flips above/below the badge to fit the window. Leaving both surfaces, pressing Escape, clicking outside or scrolling the parent closes it. Only one summary is shown at a time. Focus/Arrow Down exposes it for keyboard use; hover never steals focus. Clicking the badge still opens full history; clicking a summary version opens that exact version. File-card single/double-click behavior is unchanged.
- Loading, empty and failed states are explicit. Failure provides a manual Retry button. Requests are not fired for every visible card, and there is no automatic retry or long-lived stale cache. Closed/unmounted summaries ignore late results; workspace changes remount the badge, and backend requests validate workspace identity. Metadata changes refresh an open matching summary.

The contextual, scoped-editing decisions were checked against [Apple’s current Popovers guidance](https://developer.apple.com/design/human-interface-guidelines/popovers/): a small amount of related information, an anchor, one popover at a time, nonmodal dismissal and no accidental draft loss. The note editor is deliberately modal; the hover preview is nonmodal and has no inline edits.

## Persistence and compatibility

- Forward-only schema migration **4** adds `note` and `is_milestone` to version records with empty/unstarred defaults. No released migration is rewritten.
- Timeline reads include the metadata in their existing query, plus the workspace identity. No per-row queries, snapshot reads or AI calls are needed to display labels.
- `get_file_version_summary` performs a separate bounded, indexed, metadata-only query (`LIMIT 5`). Unlike opening full history, hover never runs physical-file reconciliation, reads snapshots or creates a version. File/workspace identity and Trash state are checked under the database lock.
- Partial writes validate workspace, file and version identity under the database lock. Deleted/trashed versions and stale requests from a switched workspace are rejected. Editing a note cannot silently clear the star, or vice versa.
- Updating annotations does not modify physical files, immutable snapshots, timestamps, search indexes, version counts or current-version pointers.
- Annotations persist across reopen and Trash/restore. Backup restoration copies both new fields; older backups without those fields restore with empty/unstarred defaults.
- Local UI updates are scoped to workspace/file/version and refresh other mounted history views. No notes are newly sent to AI services by this feature.

## Verification

- `npm test`: 166 tests passed, including exact version updates, clearing metadata, stale workspace/file/version rejection, common control integration, 50-character counting, summary positioning and eight-language parity.
- `npm run build`: typecheck and production frontend build passed. Existing bundle-size warning remains.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked`: 189 passed; two pre-existing model-asset tests ignored. Coverage includes v3-to-v4 defaults, reopen, independent note/star writes, unchanged snapshots/current pointer/counts, Trash protection, workspace isolation, and both legacy/current backup restoration. Additional coverage verifies the 50/51-character boundary (ASCII, Chinese and emoji), preservation of old longer notes and a five-row summary from 40 versions without available snapshot files.
- Isolated Chrome UI checks used the existing synthetic Markdown fixture: 2 and 40 versions, mixed Chinese/English long notes, 1280×800 light/dark and 920×640 light. Verified star, save, cancellation, failed-save/retry, centered editor, Tab loop, focus return, unchanged selection and bounded scrolling. Browser IPC was mocked; SQLite persistence was tested separately in Rust.
- Existing full-window history regression was checked at 1280×800 and 920×640 light, 1600×1000 dark, and 640×720 dark with reduced motion: independent scrolling, fixed header, split/unified Diff, menu Escape and focus restoration passed.
- Native Tauri input menus, VoiceOver and end-to-end desktop interaction in each edition remain manual acceptance. Image/PDF/plain-text integration is compiled and shares the controls; native per-format workflows are not claimed as GUI-tested here. No real user file, live database or configured AI service was used.
- Follow-up isolated Chrome checks (1280×800 light, 920×640 light, 640×720 dark) exercise the actual badge and note components with synthetic IPC: delayed loading, bounded rows/scrolling, note/star display, keyboard/Escape/focus, manual retry, clicking an exact version, 50-character save validation, preservation of longer existing notes and unchanged file double-click. Screenshot review covers dense notes and light/dark yellow-star contrast. These do not replace native Tauri acceptance.

## Manual acceptance in each edition

1. Restart the dev app after the Rust change. In an isolated folder create `test-version.md`, then save `version one`, `version two`, `version three` as separate recorded versions.
2. Open its detail view and expand version history. Star v2, add “Approved release”, close/reopen the preview, then restart the app. The star and note must remain on v2; v3 must remain current.
3. Edit and clear the note, cancel an unsaved edit, and unstar/restar v2. These operations must not create another version or change file text. Escape should close the note editor before the file preview.
4. Open full history through the version badge; confirm the same annotation, Diff and restore behavior. Check a text/image/PDF history and macOS native right-click in the note input.
5. Optionally export and restore an isolated workspace backup. The annotated version must retain its label. Community and Pro are tested separately, without sharing live application databases.
6. Save a 50-character note, then try 51 characters: the counter warns and Save is disabled; removing one character allows saving. A previously longer note must not be truncated merely by opening or cancelling the editor, or by changing its star.
7. Return to the file grid and hover the version badge. Check recent version numbers, notes and yellow stars. Move into the summary and scroll; click one version to open its exact history entry. Check Escape, outside click and rapid movement between different badges. A larger file history shows only the most recent five rows here, with all versions accessible through the full timeline.
