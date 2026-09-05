# Lume Trace Development Guide

This document describes the code paths that exist today. It is an implementation guide, not a roadmap.

## Product and storage boundary

Lume Trace is a local-first desktop file workspace. A workspace points to a user-owned physical file root. Lume Trace stores its own SQLite database and immutable version snapshots in the application data directory; these managed files must not replace the user's physical root.

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
| AI question answering | `src-tauri/src/ai_qa.rs` | Workspace retrieval, bounded prompt construction, AI-service execution, history, follow-up context, cancellation, and citations. |
| Local file metadata queries | `src-tauri/src/file_query.rs` | Indexed name/path lookup, bounded background catalog backfill, and recorded-version counts without reading document bodies. |
| File-space interface | `src/pages/file-space/` | Browser, previews, selection, drag/drop, search, settings, background status, version Diff, Trash, workspaces, and AI panel. |
| Localization | `src/shared/i18n/locales/` | Eight synchronized language dictionaries. |

## Database schema migrations

Workspace databases use SQLite `PRAGMA user_version`. The current schema version is `3`.

- A database with `user_version = 0`, including a database created before explicit schema versioning, is upgraded through migration 1 when it is opened.
- Migration 2 adds the reserved `context_json` column to AI turns; it does not by itself implement structured AI context or historical comparison in chat.
- Migration 3 creates an initially empty file-name/path lookup table and its indexes, with transactional triggers for file creation, renaming, Trash, restore, and deletion. Existing metadata is backfilled in resumable batches of 64 by the background worker, not scanned at startup. Document bodies and vectors are not involved.
- Each migration runs sequentially inside an `IMMEDIATE` transaction. Its schema changes and the new `user_version` are committed together, so a failed migration does not leave the database marked as upgraded.
- A database newer than `CURRENT_SCHEMA_VERSION` is rejected rather than opened by older code.
- Released migration numbers are append-only. Never edit or reuse an existing migration number; add a new migration and advance `CURRENT_SCHEMA_VERSION`.
- Every schema change needs tests for a fresh database, an upgrade from the previously shipped version, rollback on failure where applicable, and reopen at the resulting version.

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
- edits saved inside Lume Trace;
- same-name imports explicitly merged as the latest version;
- physical-file changes detected by the native watcher or reconciliation scan.

External changes are captured in the background and queued for a user notification. A version records immutable snapshot content and participates in historical preview, current-version switching, Diff, search extraction, and AI source attribution.

When changing file-card click, double-click, selection, or drag behavior, preserve the shared DOM and event contracts used by preview routing, manual ordering, folder moves, and native drag-out.

## AI File Assistant

The local `lookup_file_space_file_names` and `get_file_space_file_version_summary` commands expose file identity and recorded-version metadata independently of full-text or semantic extraction. Lookup reports an updating catalog until the initial background backfill is complete, preserves same-name candidates, and bounds returned results. Version counts come from actual records, not the highest version number or text inside the file. These local commands and the local historical-Diff backend are not yet connected to the AI query router; chat still uses the retrieval path below.

The current supported execution path is:

```text
question + recent workspace history
  -> indexed lexical candidate retrieval
  -> optional local semantic chunk ranking
  -> bounded source excerpts with file/version IDs
  -> selected Ollama, OpenAI-compatible LM Studio/cloud, or read-only Agent CLI invocation
  -> persisted answer, duration, and source references
```

Important boundaries:

- Ollama uses its native API: `GET /api/tags` for model discovery and `POST /api/chat` for streamed answers. Do not route it through the OpenAI-compatible adapter.
- LM Studio uses the OpenAI-compatible `GET /v1/models` and `POST /v1/chat/completions` endpoints.
- Cloud APIs use OpenAI-compatible model discovery and Chat Completions with Bearer authentication before saving, then execute through the same bounded RAG prompt path.
- Hermes and Codex are the currently supported Agent CLI answer paths. Claude Code and OpenCode have detection, configuration, and restricted invocation code, but remain **experimental** until their real end-to-end question-answering paths complete release acceptance.
- An exact-marker CLI connection check verifies the probe process only. It must not be treated as proof that retrieval, prompt delivery, streaming output, persistence, and citations all work together.
- Only retrieved excerpts are included in the selected AI-service prompt; the entire workspace is not sent.
- Follow-up questions use persisted recent turns and preferred source files. AI history is scoped to the active workspace and survives restart.
- Every Agent CLI execution requires the stored permission to be `readOnly`.
- Codex runs non-interactively in an ephemeral neutral directory with a read-only sandbox. Shell tools, web search, apps, and multi-agent execution are disabled, so it receives only the prompt built from retrieved RAG excerpts, recent conversation context, and source metadata. Host-only `CODEX_*` session and sandbox variables are removed from the child process while the user's Codex authentication directory remains available.
- Claude Code runs in print/stream-JSON mode with safe mode enabled and its tool list empty. OpenCode runs in JSON mode in a neutral directory with project instructions, external skills, default plugins, sharing, auto-update, and all tool permissions disabled. Both receive the RAG prompt through stdin rather than process arguments.
- Local-model settings and the active AI-service mode are persisted in SQLite and copied when creating or switching workspaces.

## Workspace removal safety

Removing a workspace has two explicit scopes:

1. Remove only the registry entry and leave all Lume Trace-managed data in place.
2. Remove the registry entry plus that workspace's managed SQLite files and version-snapshot directory.

The physical file root is never a managed-data deletion target. Deletion code validates canonical paths, rejects symbolic links and overlapping workspace storage, removes SQLite sidecars, and cleans the empty managed UUID directory when safe.

## Trash and recovery

Normal deletion moves a file or folder into Lume Trace Trash. Restore targets the recorded original location and requires confirmation. Manual emptying is a separate permanent operation with its own confirmation. Entries are eligible for automatic permanent purge 30 days after deletion.

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
- OCR for image-only documents;
- release-accepted Windows and Linux packages.

Claude Code and OpenCode are implemented as experimental Agent CLI adapters, not release-accepted AI answer paths.
