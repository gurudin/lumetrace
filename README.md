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
