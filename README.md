<p align="center">
  <img src="https://i.ibb.co/KxH6pYFd/logo.png" width="80" height="80" alt="Lume Trace logo">
</p>

<h1 align="center">Every file has a timeline.</h1>

<p align="center">Automatically record every save. Visually compare every change.</p>

<p align="center">
  <a href="https://i.ibb.co/67XxPbbT/1.gif">
    <img src="https://i.ibb.co/67XxPbbT/1.gif" width="960" alt="Lume Trace: edit a Markdown file from V1 to V2, see both versions in its timeline, and compare changes with inline and side-by-side Diff.">
  </a>
</p>

<p align="center">
  <a href="docs/README.zh-CN.md">简体中文</a> ·
  <a href="https://i.ibb.co/67XxPbbT/1.gif">View full-size demo</a> ·
  <a href="docs/DEVELOPMENT.md">Development guide</a>
</p>

## A timeline for your files

Lume Trace gives your local files a history you can see and use. Follow a file's timeline, compare text changes with Diff, and bring back an earlier version — without changing where your files live.

Lume Trace keeps the physical files in a folder you choose. It adds local workspace metadata, version snapshots, full-text and semantic indexes, Trash, and source-grounded AI conversations without turning the folder into a cloud drive.

The `1.0.0` scope is a free, single-user macOS application. The user confirmed the feature freeze; see the [v1.0.0 freeze record](docs/releases/v1.0.0.md). This does not indicate that a signed installer or public release has been published. Windows and Linux packages have not yet gone through release acceptance.

## What is available now

### File workspaces

- Create an empty workspace or initialize one from an existing physical folder.
- Create, rename, remove, and switch between multiple workspaces. File records, search indexes, versions, Trash, and AI history are isolated by workspace.
- Keep the physical folder under the user's ownership. Removing a workspace never silently deletes that folder.
- Browse large workspaces through bounded database queries and viewport rendering instead of loading every file into the interface.
- Exclude dot-prefixed hidden directories when initializing recursively from an existing folder.

### Files, versions, and recovery

- Create Markdown and plain-text files, import files and folders, rename or move items, assign Tags, multi-select, and drag files between folders or out to another application.
- Use adaptive-grid or list layout, manual ordering, time ordering, file preview, Quick Look-style `Space` preview, and system-app opening.
- Create an initial snapshot for imported files and detect later changes made by another application in the background.
- Inspect a file's version timeline, preview historical versions, make an older version current, and compare supported text versions with line- and word-level Diff.
- Handle a same-folder, same-name import explicitly: merge it as the latest version or keep it under a new name. Identical content is skipped and the existing file can be located directly.
- Move deleted items to Trash, restore them to their recorded location, empty Trash after confirmation, and automatically purge entries after 30 days.
- Export and restore Lume Trace backups. Backup and application-specific migration boundaries are documented in [docs/MIGRATION.md](docs/MIGRATION.md).

### Search and local knowledge index

- Open global search with `Command + K` on macOS (`Alt + K` is implemented for Windows and Linux keyboard behavior).
- Search indexed file names, extracted body text, and Tags with SQLite FTS5.
- Extract readable text asynchronously after import; file registration does not wait for content extraction or semantic indexing.
- Extract text from PDF, DOCX, XLSX, PPTX, Markdown, TXT, source code, and common structured-text formats. Image-only documents require OCR and are not currently supported.
- Optionally download the Multilingual E5 Small model from Hugging Face and build a local semantic index. Extracted text, chunks, embeddings, and the ANN index remain on the Mac.
- Inspect background extraction and semantic-index work, including progress, pause/resume, failure details, and retry.

### Lumie · AI File Assistant

Lumie routes natural-language questions over the active workspace:

1. The selected AI service interprets the question using bounded recent context and file identities.
2. File/version questions use local metadata queries. Content questions use bounded RAG passages. Historical comparisons read only the requested snapshots and send their local Diff for summarization.
3. The answer, references, and target file/version-range context are stored in the active workspace. Version counts come directly from recorded database rows.

It does not ask a model to open and scan every file. Conversation history survives application restarts and can be used for follow-up questions.

#### AI service support

| Service | Interface used by Lume Trace | Status |
| --- | --- | --- |
| Ollama | Native `GET /api/tags` and `POST /api/chat` | Supported |
| LM Studio | OpenAI-compatible `GET /v1/models` and `POST /v1/chat/completions` | Supported |
| Cloud API | OpenAI-compatible model discovery and Chat Completions with a locally stored API key | Supported |
| Hermes CLI | Non-interactive, read-only RAG prompt | Supported |
| Codex CLI | Non-interactive, read-only sandbox with only the prepared RAG prompt | Supported |
| Claude Code | Restricted non-interactive RAG prompt | **Experimental** |
| OpenCode | Restricted non-interactive RAG prompt | **Experimental** |

AI configuration is optional. File browsing, version tracking, full-text search, and the local semantic index do not require an answer model. A CLI connection check is not the same as an end-to-end answer guarantee; the two experimental CLI paths still require broader real-environment acceptance.

### macOS experience

- Use light, dark, or system-following appearance with the selected accent color applied consistently.
- Use familiar macOS selection, keyboard, drag-and-drop, contextual-menu, sheet, and preview behavior.
- Choose among Simplified Chinese, Traditional Chinese, English, Japanese, Korean, German, French, and Spanish.

## Local data and privacy boundary

- Physical files stay in the selected workspace folder.
- Workspace metadata, versions, extracted text, indexes, Trash records, and AI history are stored locally.
- Installing the E5 model downloads model files but does not upload workspace content.
- On a user-initiated AI query, the question, bounded recent context, target file identities, and relevant excerpts or requested version Diff may be sent to the selected local service, cloud endpoint, or Agent CLI. The entire workspace is not sent as one request.
- Cloud API credentials are stored in the local workspace database and excluded from exported backups. Backup archives themselves are not encrypted.

## Current boundaries

The first free release does not provide:

- team accounts, member permissions, sharing, or real-time collaboration;
- NAS synchronization, cloud synchronization, or cross-device conflict resolution;
- Eagle-specific or other application-private database migration;
- OCR for image-only PDFs or images;
- release-accepted Windows or Linux installers.

Claude Code and OpenCode are visible as experimental integrations; they should not be presented as fully accepted execution paths yet.

## First run

1. Create a workspace and choose a new or existing physical folder.
2. For an existing folder, let registration finish; content extraction continues as background work.
3. Use `Command + K` for full-text search. Install the optional E5 model in Semantic Search if meaning-based recall is needed.
4. Configure one supported AI service only if you want source-grounded questions, summaries, or analysis.

## Development

Requirements:

- Node.js and npm;
- Rust toolchain;
- Tauri 2 prerequisites for macOS;
- an AI service only when testing the optional AI question-answering path.

```bash
npm install
npm run tauri:dev
```

Run development builds with `npm run tauri:dev`; do not install them into `/Applications`.

Useful checks:

```bash
npm test
npm run typecheck
npm run build
cd src-tauri
cargo fmt --check
cargo test --lib
cargo check
```

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for code boundaries, database migrations, background processing, and verification guidance.
