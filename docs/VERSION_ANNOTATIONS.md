# Version notes and milestone stars

These are shared **Community and Pro** capabilities, explicitly confirmed by the user. They are not gated paid features. Pro consumes the same implementation through its pinned Community dependency; the editions still keep separate application data.

## Interaction

- Every version row in Markdown, plain-text, image, PDF and full-window history has a star and a note action. Selecting a version, viewing Diff and restoring a version keep their existing behavior.
- A star marks that specific version as a milestone. It can be removed independently of the note. Starring never changes the current-version pointer or chronological ordering.
- Notes are shown below the version details, with a two-line preview and full text available in the editor. Empty notes show “Add note”. This is a lightweight milestone annotation, not a Git tag namespace or multi-tag system.
- The note editor is a short, centered modal task with Save/Cancel, a 1000-character input bound, visible focus and native input context menus. Escape cancels only the note editor; Command/Ctrl+Enter or Command/Ctrl+S saves. IME composition does not trigger these shortcuts. Clicking outside does not discard the draft.
- Pending writes disable duplicate operations. A failed write preserves the draft and keeps the editor open for retry. Focus returns to the invoking note control after the dialog is removed and the control re-enabled.
- Version selection and metadata buttons are siblings, not nested interactive elements. Current selection, scroll ownership and dark image-preview contrast are preserved. Eight supported languages have matching copy.
- The planned hover summary is not included in this change; it can later reuse the same metadata.

The contextual, scoped-editing decisions were checked against [Apple’s current Popovers guidance](https://developer.apple.com/design/human-interface-guidelines/popovers/): a small related task, explicit save/cancel when meaningful, and no accidental draft loss. The note editor is deliberately modal; a future hover preview should remain nonmodal.

## Persistence and compatibility

- Forward-only schema migration **4** adds `note` and `is_milestone` to version records with empty/unstarred defaults. No released migration is rewritten.
- Timeline reads include the metadata in their existing query, plus the workspace identity. No per-row queries, snapshot reads or AI calls are needed to display labels.
- Partial writes validate workspace, file and version identity under the database lock. Deleted/trashed versions and stale requests from a switched workspace are rejected. Editing a note cannot silently clear the star, or vice versa.
- Updating annotations does not modify physical files, immutable snapshots, timestamps, search indexes, version counts or current-version pointers.
- Annotations persist across reopen and Trash/restore. Backup restoration copies both new fields; older backups without those fields restore with empty/unstarred defaults.
- Local UI updates are scoped to workspace/file/version and refresh other mounted history views. No notes are newly sent to AI services by this feature.

## Verification

- `npm test`: 163 tests passed, including exact version updates, clearing metadata, stale workspace/file/version rejection, common control integration and eight-language parity.
- `npm run build`: typecheck and production frontend build passed. Existing bundle-size warning remains.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked`: 187 passed; two pre-existing model-asset tests ignored. Coverage includes v3-to-v4 defaults, reopen, independent note/star writes, unchanged snapshots/current pointer/counts, Trash protection, workspace isolation, and both legacy/current backup restoration.
- Isolated Chrome UI checks used the existing synthetic Markdown fixture: 2 and 40 versions, mixed Chinese/English long notes, 1280×800 light/dark and 920×640 light. Verified star, save, cancellation, failed-save/retry, centered editor, Tab loop, focus return, unchanged selection and bounded scrolling. Browser IPC was mocked; SQLite persistence was tested separately in Rust.
- Existing full-window history regression was checked at 1280×800 and 920×640 light, 1600×1000 dark, and 640×720 dark with reduced motion: independent scrolling, fixed header, split/unified Diff, menu Escape and focus restoration passed.
- Native Tauri input menus, VoiceOver and end-to-end desktop interaction in each edition remain manual acceptance. Image/PDF/plain-text integration is compiled and shares the controls; native per-format workflows are not claimed as GUI-tested here. No real user file, live database or configured AI service was used.

## Manual acceptance in each edition

1. Restart the dev app after the Rust change. In an isolated folder create `test-version.md`, then save `version one`, `version two`, `version three` as separate recorded versions.
2. Open its detail view and expand version history. Star v2, add “Approved release”, close/reopen the preview, then restart the app. The star and note must remain on v2; v3 must remain current.
3. Edit and clear the note, cancel an unsaved edit, and unstar/restar v2. These operations must not create another version or change file text. Escape should close the note editor before the file preview.
4. Open full history through the version badge; confirm the same annotation, Diff and restore behavior. Check a text/image/PDF history and macOS native right-click in the note input.
5. Optionally export and restore an isolated workspace backup. The annotated version must retain its label. Community and Pro are tested separately, without sharing live application databases.
