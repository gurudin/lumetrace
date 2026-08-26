# LumeTrace

LumeTrace is a local-first, traceable file workspace for solo operators and small teams.

The initial application is extracted from Virelume's File Space and focuses on:

- local file and folder management;
- previews for images, PDF, Markdown, and text;
- tags and full-text search;
- file version history and traceable changes;
- user-owned storage without an agent-orchestration runtime.

## Development

```bash
npm install
npm run tauri:dev
```

The desktop application uses Tauri 2, Rust, React, TypeScript, SQLite, and FTS5.

## Workspace initialization

- **Create File Workspace** sets a storage folder and starts with an empty index.
- **Import Existing Folder** scans the physical folder hierarchy, registers existing files in place, and creates an initial v1 snapshot for each file. It does not read another application's SQLite database.

Dedicated one-click importers for mainstream file managers such as Eagle are a later product direction. Those importers will remain separate from the generic physical-folder initialization flow.
