# Lume Trace

**A local-first file workspace with automatic version history, full-text and semantic search, and source-grounded AI answers.**

[简体中文](docs/README.zh-CN.md) · [Development guide](docs/DEVELOPMENT.md) · [Import and migration boundary](docs/MIGRATION.md)

Lume Trace helps individuals and small teams organize files without giving up ownership of the physical files. Files remain in a user-selected folder, while Lume Trace maintains local metadata, version snapshots, search indexes, Trash, and AI conversation history.

## Available today

### Local-first file workspaces

- Create an empty workspace or initialize one from an existing physical folder.
- Keep multiple custom-named workspaces and switch between them.
- Isolate each workspace's database, version snapshots, search index, Trash, and AI history.
- Preserve the physical file root when removing a workspace. The user separately chooses whether Lume Trace's managed database and version data should also be deleted.
- Export a workspace backup and restore it into the current workspace at a newly created physical root; the previous physical root remains on disk.

### Traceable file versions

- Create an initial version for every imported file.
- Detect file changes made by external applications and create new versions in the background.
- Notify the user when an external edit becomes a new version and link directly to its timeline.
- Browse version history, preview historical content, switch the current version, and compare text versions with line- and word-level diffs.
- Resolve same-name imports explicitly by adding the incoming file as the latest version or keeping it as a separately renamed file. Identical files are skipped and can be located directly.

### Search and local knowledge indexing

- Open global search with `Command + K` on macOS or `Alt + K` on Windows and Linux.
- Search indexed filenames, extracted body text, and Tags with SQLite FTS5.
- Optionally install the bundled multilingual embedding model for local semantic ranking; embeddings and indexes stay on the device.
- Extract readable content asynchronously after import so the file operation can finish without waiting for indexing.
- Extract text from PDF, DOCX, XLSX, PPTX, Markdown, plain text, source code, and common structured-text formats.
- Inspect, pause, resume, and retry content extraction and semantic-index background work.

### Lumie · AI File Assistant

- Ask questions across the current file workspace instead of searching one file at a time.
- Retrieve a bounded set of relevant local excerpts, send only those excerpts to the selected AI service, and show the referenced files below the answer.
- Preserve file and version references with each answer.
- Store conversation history per workspace and include recent context in follow-up questions, including after an application restart.
- Run questions through either an OpenAI-compatible local model service (Ollama or LM Studio) or a configured Hermes Agent CLI with read-only file permission.

### File management and recovery

- Browse files in adaptive-grid or list layouts with pagination and viewport virtualization for large workspaces.
- Create Markdown and TXT files; rename, move, multi-select, drag between folders, drag out to other applications, Tag, preview, and open files with system applications.
- Preview images, PDF, Markdown, and text inside Lume Trace.
- Move deleted items to Trash, restore them to their original locations after confirmation, or empty Trash manually after a second confirmation.
- Automatically purge Trash entries 30 days after they were deleted.

### Desktop experience

- Light, dark, and system-following appearances.
- Eight interface languages: Simplified Chinese, Traditional Chinese, English, Japanese, Korean, German, French, and Spanish.
- macOS-oriented desktop interaction built with Tauri 2, while retaining Windows and Linux keyboard behavior where implemented.

## Current boundaries

The following are **not current product capabilities**:

- team accounts, permissions, real-time collaboration, NAS synchronization, or cloud synchronization;
- direct cloud OpenAI-compatible API execution in the AI File Assistant;
- AI execution through Claude Code, Codex CLI, or OpenCode (their local installation and configuration can be detected, but file-space Q&A currently runs through local models or Hermes only);
- one-click migration from Eagle or another application's private database;
- OCR for image-only documents.

## How the data flow works

1. Lume Trace registers a physical file and returns control to the interface.
2. Background workers extract readable text and update the local FTS5 index.
3. If the optional semantic model is installed, the worker also creates local embeddings.
4. Search queries use indexed candidates instead of loading or scanning every file in the interface.
5. AI questions retrieve relevant excerpts from the current workspace before invoking the selected local model or Hermes, then persist the answer and its sources locally.

## Development

Requirements:

- Node.js and npm
- Rust toolchain
- Tauri 2 platform prerequisites
- An OpenAI-compatible local model service (Ollama or LM Studio), or Hermes Agent CLI, when testing AI question answering

```bash
npm install
npm run tauri:dev
```

Development builds must be run with `npm run tauri:dev`; do not install them into `/Applications`.

Useful checks:

```bash
npm test
npm run typecheck
npm run build
cd src-tauri && cargo test --lib && cargo check && cargo fmt --check
```

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for module boundaries, background processing, and verification guidance.
