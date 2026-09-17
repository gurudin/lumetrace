# LumeTrace Development Guide

This document describes the code paths that exist today. It is an implementation guide, not a roadmap.

## Product and storage boundary

LumeTrace is a local-first desktop file workspace. A workspace points to a user-owned physical file root. LumeTrace stores its own SQLite database and immutable version snapshots in the application data directory; these managed files must not replace the user's physical root.

Each workspace has an independent:

- SQLite database;
- version-snapshot directory;
- file, folder, Tag, Trash, and manual-order records;
- extracted-text, FTS5, and semantic-index records;
- AI turns and source references.

The selected AI service settings and installed semantic-model setting are copied when a new workspace is created because they are application-level service choices. Workspace content and AI history remain isolated.

## Main implementation areas

| Area | Main files | Current responsibility |
| --- | --- | --- |
| Desktop commands and startup | `src-tauri/src/lib.rs` | Registers Tauri commands, background workers, watchers, and plugins. |
| File operations and versions | `src-tauri/src/file_space.rs` | Physical-file operations, snapshots, version timeline, watcher, Trash, backup, import, search commands, and background status. |
| Workspace isolation | `src-tauri/src/workspace.rs` | Workspace registry, creation, switching, rename, removal, and managed-data safety checks. |
| Content extraction | `src-tauri/src/content_extractor.rs` | Plain-text, PDF, DOCX, XLSX, and PPTX text extraction with size limits. |
| Semantic indexing | `src-tauri/src/semantic_search.rs` | Optional model installation, embedding queue, semantic ranking, status, pause, and retry. |
| Agent CLI configuration | `src-tauri/src/agent_cli.rs` | CLI discovery, health checks, status classification, and persisted service settings. |
| AI service adapters | `src-tauri/src/ai_service.rs` | Native Ollama, OpenAI-compatible LM Studio/cloud requests, model discovery, streaming responses, and persisted settings. |
| AI question answering | `src-tauri/src/ai_qa.rs`, `src-tauri/src/ai_query.rs` | Semantic query planning, read-only file/version routing, bounded RAG, shared AI-service execution, persisted file/range context, cancellation, and citations. |
| Local file metadata queries | `src-tauri/src/file_query.rs` | Indexed name/path lookup, bounded background catalog backfill, and recorded-version counts without reading document bodies. |
| File-space interface | `src/pages/file-space/` | Browser, previews, selection, drag/drop, search, settings, background status, version Diff, Trash, workspaces, and AI panel. |
| Localization | `src/shared/i18n/locales/` | Eight synchronized language dictionaries. |

## Database schema migrations

Workspace databases use SQLite `PRAGMA user_version`. The current schema version is `4`.

- Migration 4 adds per-version `note` and `is_milestone` metadata with empty/unstarred defaults. It does not create snapshots or change current-version pointers, timestamps, file contents or indexes. These labels are shared Community/Pro capabilities and are included in workspace database backups.

- A database with `user_version = 0`, including a database created before explicit schema versioning, is upgraded through migration 1 when it is opened.
- Migration 2 adds the reserved `context_json` column to AI turns; it does not by itself implement structured AI context or historical comparison in chat.
- Migration 3 creates an initially empty file-name/path lookup table and its indexes, with transactional triggers for file creation, renaming, Trash, restore, and deletion. Existing metadata is backfilled in resumable batches of 64 by the background worker, not scanned at startup. Document bodies and vectors are not involved.
- Each migration runs sequentially inside an `IMMEDIATE` transaction. Its schema changes and the new `user_version` are committed together, so a failed migration does not leave the database marked as upgraded.
- A database newer than `CURRENT_SCHEMA_VERSION` is rejected rather than opened by older code.
- Released migration numbers are append-only. Never edit or reuse an existing migration number; add a new migration and advance `CURRENT_SCHEMA_VERSION`.
- Every schema change needs tests for a fresh database, an upgrade from the previously shipped version, rollback on failure where applicable, and reopen at the resulting version.

## File browsing and import behavior

- Create Markdown and plain-text files, import files and folders, rename or move items, assign Tags, multi-select, and drag files between folders or out to another application.
- Adaptive-grid and list layouts support manual ordering, time ordering, file preview, Quick Look-style `Space` preview, and system-app opening. Large workspaces use bounded database queries and viewport rendering rather than loading the entire file catalog into the interface.
- A same-folder, same-name import requires an explicit choice: merge it as the latest version or keep it under a new name. Identical content is skipped, and the existing file can be located directly.
- Global search uses `Command + K` on macOS. `Alt + K` keyboard behavior exists for Windows and Linux, but installers for those platforms are not release-supported.
- Full-text extraction supports PDF, DOCX, XLSX, PPTX, Markdown, TXT, source code, and common structured-text formats, subject to the extractor's resource limits. OCR for image-only content is not implemented.
- The optional semantic model is Multilingual E5 Small, downloaded from Hugging Face. Extracted text, chunks, embeddings, and the ANN index remain local during indexing. Background status exposes progress, pause/resume, failure details, and retry controls.
- Backup export and restoration, including replacement of the current workspace's managed records and version store, are documented in [MIGRATION.md](MIGRATION.md). Backup archives are not encrypted. Cloud API keys are stored locally as unencrypted configuration and excluded from completed exports.

## Import and indexing pipeline

File registration and knowledge indexing are intentionally separate:

1. Create or import registers the physical hierarchy and initial file versions.
2. The user-facing file operation completes without waiting for content extraction.
3. A background worker extracts readable text from supported formats.
4. The FTS5 document is updated from the file name, extracted body, Tags, and available trace fields.
5. If the optional semantic model is installed, a queued worker creates embeddings from the extracted content.
6. Background status exposes queued, running, completed, failed, paused, and retry states.

Do not move extraction, embedding generation, or recursive whole-workspace work onto the main UI thread. Import progress must remain observable, and large queries must remain bounded.

Hidden directories whose names start with `.` are excluded during recursive folder initialization. The generic importer reads physical files only; it does not inspect another application's SQLite database.

## Search pipeline

Search is index-backed:

1. The frontend sends a query and selected scopes to `search_file_space_files`.
2. SQLite FTS5 produces a bounded lexical candidate set from filename, body text, and Tags.
3. When the local semantic model is available, semantic scores can rerank the bounded candidate set.
4. The frontend receives identifiers and scores, then renders only the required file page/viewport.

Never reintroduce whole-workspace loading, unbounded result returns, or one-DOM-node-per-file rendering. The existing paginated backend and viewport virtualization are stability requirements, not optional optimizations.

## Version tracking

Version creation currently comes from:

- initial registration during existing-folder import;
- edits saved inside LumeTrace;
- same-name imports explicitly merged as the latest version;
- physical-file changes detected by the native watcher or reconciliation scan.

External changes are captured in the background and queued for a user notification. A version records immutable snapshot content and participates in historical preview, current-version switching, Diff, search extraction, and AI source attribution.

When changing file-card click, double-click, selection, or drag behavior, preserve the shared DOM and event contracts used by preview routing, manual ordering, folder moves, and native drag-out.

## AI File Assistant

The local `lookup_file_space_file_names` and `get_file_space_file_version_summary` commands expose file identity and recorded-version metadata independently of full-text or semantic extraction. AI queries use the same local lookup and metadata helpers. Lookup reports an updating catalog until the initial background backfill is complete, preserves same-name candidates, and bounds returned results. Version counts come from actual records, not the highest version number or text inside the file.

The current supported execution path is:

```text
question + bounded recent questions + persisted file/range context
  -> selected AI service produces a validated, read-only JSON query plan
  -> name/path/context ID resolves the target (ambiguity asks for clarification)
     -> file/version metadata: indexed SQL, deterministic answer, no body or vector lookup
     -> historical comparison: read selected snapshots, compute bounded local Diff, summarize
     -> content question: bounded lexical/semantic passage retrieval, summarize
  -> persist answer, duration, sources, workspace/file IDs and version range
```

Important boundaries:

- Ollama uses its native API: `GET /api/tags` for model discovery and `POST /api/chat` for streamed answers. Do not route it through the OpenAI-compatible adapter.
- LM Studio uses the OpenAI-compatible `GET /v1/models` and `POST /v1/chat/completions` endpoints.
- Cloud APIs use OpenAI-compatible model discovery and Chat Completions with Bearer authentication before saving, then use the same query router as local models and Agent CLIs.
- Hermes and Codex are the currently supported Agent CLI answer paths. Claude Code and OpenCode have detection, configuration, and restricted invocation code, but remain **experimental** until their real end-to-end question-answering paths complete release acceptance.
- An exact-marker CLI connection check verifies the probe process only. It must not be treated as proof that retrieval, prompt delivery, streaming output, persistence, and citations all work together.
- Only user-initiated queries invoke the selected service. Planning sends the current question, up to six bounded recent questions, and up to eight target identities/ranges, not the full catalog or document bodies. Content answers send retrieved excerpts; requested version comparisons send only the selected Diff, with bounded recent conversation context. The entire workspace and all historical snapshots are never sent as one request.
- Follow-up file IDs and version ranges are stored with the completed turn in `context_json`. IDs are revalidated against the pinned workspace; renamed files retain identity, deleted files are not substituted, and new explicit targets replace previous ones. Actual counts are re-queried each time. Retrying an older turn does not inherit later context.
- Invalid plans fail explicitly without silently falling back to RAG. Counts need one model planning call and a local database query; content and Diff answers use an additional model call. Historical versions are not re-embedded. Snapshot/resource failures do not fall back to another file.
- Every Agent CLI execution requires the stored permission to be `readOnly`.
- Codex runs non-interactively in an ephemeral neutral directory with a read-only sandbox. Shell tools, web search, apps, and multi-agent execution are disabled, so it receives only the application's bounded planning or answer prompt. Host-only `CODEX_*` session and sandbox variables are removed from the child process while the user's Codex authentication directory remains available.
- Claude Code runs in print/stream-JSON mode with safe mode enabled and its tool list empty. OpenCode runs in JSON mode in a neutral directory with project instructions, external skills, default plugins, sharing, auto-update, and all tool permissions disabled. Both receive the RAG prompt through stdin rather than process arguments.
- Local-model settings and the active AI-service mode are persisted in SQLite and copied when creating or switching workspaces.

Query-route regression tests use synthetic snapshots and isolated databases. Loopback fixtures exercise the real cloud, LM Studio, and native Ollama stream adapters without reading saved service configuration or contacting a real provider. Real-model intent recognition and the native chat interaction still require manual acceptance. Keep the existing request timer and cancellation control across planning, locating, version queries, comparisons, retrieval, and generation; report only the phase actually running.

## Workspace removal safety

Removing a workspace has two explicit scopes:

1. Remove only the registry entry and leave all LumeTrace-managed data in place.
2. Remove the registry entry plus that workspace's managed SQLite files and version-snapshot directory.

The physical file root is never a managed-data deletion target. Deletion code validates canonical paths, rejects symbolic links and overlapping workspace storage, removes SQLite sidecars, and cleans the empty managed UUID directory when safe.

## Trash and recovery

Normal deletion moves a file or folder into LumeTrace Trash. Restore targets the recorded original location and requires confirmation. Manual emptying is a separate permanent operation with its own confirmation. Entries are eligible for automatic permanent purge 30 days after deletion.

Changes to this area must keep database state and physical files transactionally consistent and preserve recovery behavior after interruption.

## Desktop UI rules

Read `AGENTS.md` before changing a screen or interaction. Every UI change must be checked against current macOS conventions, including light/dark appearance, keyboard focus, Escape behavior, reduced motion, native-sized controls, selection, drag behavior, scrolling ownership, and dense realistic data.

The interface supports eight synchronized locales:

`zh`, `zhTW`, `en`, `ja`, `ko`, `de`, `fr`, and `es`.

Any user-facing string must be added to all eight dictionaries. Run the localization parity test before claiming completion.

The current sidebar interaction keeps Favorites and My Folders in one scroll owner, alternates the sticky group heading at the section boundary, and supports recursive expand-all/collapse-all for folder subtrees. Preserve these behaviors when changing navigation layout or folder disclosure.

## Development and verification

Run the desktop app only in development mode:

```bash
npm install
npm run tauri:dev
```

Do not install a development build into `/Applications`.

Run the checks appropriate to the change:

```bash
npm test
npm run typecheck
npm run build
cd src-tauri
cargo fmt --check
cargo test --lib
cargo check
```

OS-owned interactions such as Finder-to-app drag/drop, app-to-app drag-out, native file dialogs, and macOS permission prompts require a short manual verification when reliable automation is unavailable.

## Not implemented yet

- team identity, roles, permissions, sharing, or concurrent collaboration;
- NAS or cloud synchronization and conflict resolution;
- Eagle-specific or other application-specific database migration;
- image OCR on non-Apple platforms and downloadable advanced visual models;
- release-accepted Windows and Linux packages.

Claude Code and OpenCode are implemented as experimental Agent CLI adapters, not release-accepted AI answer paths.

## Apple on-device content recognition

On macOS 10.15+, the shared native extractor uses Vision OCR and image classification; older systems retain ordinary PDF text extraction. No model, Python environment, source upload, or Plugin is required. Supported raster extensions are PNG, JPEG, WebP, GIF, BMP, TIFF, HEIC and HEIF, subject to the OS decoder; image containers use the first frame. PDFKit preserves the text layer, and OCRs textless pages and pages containing raster images (including nested Form resources and inline images). Same-page body text no longer prevents embedded-image recognition. Spatially overlapping duplicate text lines are suppressed. Recognition rasters are limited to 3200 pixels on the longest edge, and the existing 100 MiB source / five-million-character extraction bounds remain. A native request times out after 120 seconds and reports failure through existing background-task retry; no additional native worker starts while the timed-out request is still finishing.

Results are stored in the existing SQLite document index. Schema migration 5 adds optional cached `visual_json`; deployed v4 text and FTS entries are preserved without a synchronous rebuild. Normalized top-left rectangles include crop/rotation transforms and UTF-16 substring ranges. Geometry is limited to 50,000 detailed character boxes per document, with recognized-line fallback. Search preview renders the actual image or selected PDF page, highlights cached matching regions, and loads only the selected source. Query/hit changes and resizing do not re-recognize the file. Preview failures have a retry action; stale file/revision responses cannot provide overlays. Opening the separate full-detail viewer retains its previous behavior; these overlays belong to the search preview.

Inferred categories retain explicit provenance for indexing and a small eight-language common-object vocabulary; internal markers and multilingual aliases are excluded from visible snippets. Categories never change filenames, tags or originals, and category matches do not invent object-location boxes. E5, when installed separately, consumes the recognized text through the existing index pipeline. The existing unicode61 tokenizer is unchanged: joined Chinese words may not match a partial token if OCR omits spaces; this milestone does not add Chinese word segmentation.

Extractor revision 3 affects images/PDF only; unchanged text/Office caches remain revision 1. Local upgrade repair is bounded and resumable. Replica preparation also compares extraction revision, not only remote file revision. Extraction runs outside file-operation/database locks, and local commits validate workspace, file stamp, path and document generation. Replica commits validate the caller's revision. Editing invalidates old geometry in the same index write; metadata-only updates preserve valid geometry.

Automatable checks (2026-09-17): 221 frontend tests and production build passed; 207 default Rust tests passed with seven explicit opt-in tests skipped. Four native recognition regressions exercise English/Chinese PNG/JPEG/TIFF, scanned/mixed/same-page embedded-image PDFs, rotation/cropping, classification, unchanged source hashes, cached geometry, local/replica search, restart persistence and stale-result rejection. Eight synthetic Chrome combinations (light/dark, Chinese/English, 920×640/1280×800) use real native-index output to check that highlight rectangles cover rendered text pixels, including rotated PDF pages, and test dense scrolling, source failure/retry, delayed replies, resize caching, compact filename-only/empty search and retained queries. Pixel checks caught and fixed CoreGraphics' downscale-only drawing transform; 2x OCR now applies explicit scale after the 1x crop/rotation transform. These checks do not access live remote storage or the user's application data.

Reproduce with a fresh temporary directory (the generator refuses existing output files):

```sh
xcrun clang -fobjc-arc -fmodules scripts/vision-fixtures.m -framework AppKit -framework CoreText -framework ImageIO -framework PDFKit -o /tmp/lumetrace-vision-fixtures
vision_test_dir=$(mktemp -d /tmp/lumetrace-vision-fixtures.XXXXXX)
/tmp/lumetrace-vision-fixtures "$vision_test_dir"
LUMETRACE_TEST_VISION_DIR="$vision_test_dir" LUMETRACE_TEST_WRITE_GEOMETRY=1 cargo test --manifest-path src-tauri/Cargo.toml --lib --locked native_vision -- --ignored --nocapture --test-threads=1
# In a separate terminal; this fixture configuration never mounts the real App.
npm run dev -- --config tests/browser/vite.config.ts --host 127.0.0.1 --port 1432 --strictPort
# With Playwright available; optional LUMETRACE_PLAYWRIGHT_PATH selects an existing package.
LUMETRACE_TEST_VISION_DIR="$vision_test_dir" node tests/browser/searchVisual.cjs
```

Manual acceptance remains: import an image/scanned PDF, wait for Background Tasks, search a visible word or common object, inspect the search preview, open the result, restart and repeat. Use synthetic data for connected storage, verify unchanged files are not re-downloaded, and check pause/retry while recognition is pending. Native WKWebView transport, older macOS versions, every image codec/orientation, pathological PDFs, and broad photo-classification accuracy remain manual boundaries. No running application, user database, model installation or external storage was accessed during the isolated tests.
