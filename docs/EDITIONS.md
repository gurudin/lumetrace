# Community, Pro and Team editions

## Boundary

LumeTrace uses one fixed commercial boundary:

> **Free manages files on this computer. Pro manages files in my self-owned storage. Team manages files in our shared self-owned storage.**

The short labels are **Free = Local**, **Pro = Personal Anywhere**, and **Team = Shared Anywhere**.

Free and Pro have the same non-Plugin core feature set. The split must not remove, limit or charge for a local core capability merely to manufacture Pro value.

| Ownership | Repository |
| --- | --- |
| File spaces and physical files; automatic snapshots, Timeline, Diff and restore | `lumetrace` |
| Previews/editing, import/export, trash, tags and file operations | `lumetrace` |
| Filename/full-text/semantic search; E5 management; current AI providers, chat and version queries | `lumetrace` |
| Shared settings, localization, accessibility, UI/UX, bug/security/performance fixes | `lumetrace` |
| Official application entry, packaging and commercial licensing | Private `lumetrace-pro` |
| Native personal NAS/S3/OSS/MinIO connections, remote access and multi-device coordination | Pro in private `lumetrace-pro` |
| Team permissions, collaboration, shared customer-owned storage, members and audit | Team in private `lumetrace-pro` |
| Planned Official Plugin system and tier-specific Plugin entitlements | Private `lumetrace-pro`; not implemented |

Community is a complete application, not a trial. A new paid capability does not justify moving a pre-existing feature or its maintenance into Pro. Confirm unclear feature boundaries before implementation.

Free can open an operating-system-mounted NAS, iCloud, Dropbox or similar directory as an ordinary local folder. This does not include LumeTrace-managed credentials, remote endpoints, failover, synchronization or a cross-device guarantee.

Pro is a paid single-user license for one person using self-owned storage across devices. Its commercial model is a one-time purchase unless later changed; no exact price is fixed. Its only core product differences from Free are native connected-storage management and the Plugin entitlements declared for the Pro tier. Advanced Diff, automation, batch workflows and other ordinary built-in features do not form a Free/Pro boundary.

Team is a subscription. It includes all Pro capabilities and adds multi-user shared workspaces, members/seats, invitations, permissions, roles, edit locks, conflict handling, modification identity, activity/audit logs and team administration.

LumeTrace does not host user files or provide a NAS relay. Users own their storage, credentials and network reachability. Each device keeps its own SQLite database, cache, full-text index and vector index. Agreed `.lumetrace` metadata supports synchronization without turning LumeTrace's licensing service into file or credential hosting.

The future Official Plugin system is planned but not implemented. Each Plugin declares its allowed tiers, such as `Free / Pro / Team` or `Pro / Team`. Never remove a Free core capability and repackage it as a Plugin.

Version notes and milestone stars are explicitly shared Community/Pro features. Their UI, storage and migrations belong in Community and are reused by Pro, without payment checks.

## One implementation, two entries

The current layout stays in place to avoid a disruptive directory migration:

- `src/bootstrap.tsx` mounts the shared `App`, providers, translations, styles and native menu behavior. Community `src/main.tsx` and the Pro entry call it.
- `build/createViteConfig.ts` reuses the public `index.html`, including the early theme/loading surface, with edition-owned name and asset paths. No second copy of startup behavior is maintained.
- `src-tauri` exports `lumetrace_lib::run(context)` and `run_with_plugins(context, plugins)`. All common commands, workers, previews and lifecycle handling remain here. The calling binary creates its own Tauri context.
- Pro references an exact public commit through `vendor/lumetrace`, a Git submodule. Its build uses the pinned public npm lockfile/toolchain and Rust library, not source-file copies or a floating branch dependency.

`run_with_plugins` is an internal extension seam. It is not a sandbox, a stable third-party plugin API or a plugin marketplace. Professional commands must be namespaced plugins, not replacement common handlers.

`mountLumeTrace(element, extension?)` also accepts an optional edition-owned page and entry component. The shared first-run page and settings menu render that same entry when supplied; Community supplies nothing and its UI stays unchanged. The extension page participates in the shared theme/language shell without requiring a personal workspace. Navigating into it hides/inerts, but does not unmount, the workspace and its background work. Shared search shortcuts and file drops do not act on the hidden workspace. Page contents, authorization and professional service calls remain outside the public source.

## Fix and release workflow

An optional `WorkspaceProvider` supplies edition-owned choices and a data source, **not a second file page**. The existing `FileSpacePage`, folder tree, list/grid and inspector remain shared. The bottom-left workspace name opens an undimmed, keyboard-accessible grouped popover; settings still manages local spaces. The provider resolves the initial/remembered selection before mounting the workbench. Source changes remount file/preview state and remember folders by selection key. Management pages remain separate from the file workbench.

`useWorkspaceInvoke` binds each component's file commands to its selected source. Unsupported external commands fail instead of falling through to the local database; stale replies and callbacks after unmount are rejected. Application settings and local registry management remain native. This frontend routing is not an authorization boundary: every external native operation must independently validate access and credentials. Declared unavailable capabilities disable their controls; the seam itself does not implement remote writes, previews, history, indexes or synchronization. Those data paths must be implemented and verified by the edition before being enabled.

1. Implement and test a common change in public `develop`; commit in English and push.
2. In private `develop`, fetch that public commit and update the submodule's exact commit reference. Never edit tracked files inside the Pro submodule.
3. Run Pro integration checks against the new reference, review the diff and commit/push the reference update.
4. Test and release each edition independently. Updating the public branch alone does not change an existing Pro checkout or binary. Never automatically merge/push `main` or create releases.

The public build and CI must never require private repository access. Pro's own tests check repository boundaries and shared integration; common behavior tests remain public. Do not copy tests or fixes between repositories.

## Data safety

Community keeps `com.lumetrace.desktop` and all existing storage identifiers unchanged. Pro uses `com.lumetrace.pro.desktop`, a distinct application-data directory, webview store and dev port. It must not automatically discover, open or migrate the community database, credentials or version store.

Both editions can still edit the same physical folder if a user explicitly selects it. Separate application data does not make concurrent physical-file edits safe. Use different synthetic folders when testing both editions. No cross-edition live database sharing or automatic workspace import is provided in this foundation.

Base database migrations are owned here, currently starting from the released baseline documented in `DEVELOPMENT.md`. Add forward-only migrations; never copy/rewrite them in Pro. Future paid schema must have independent ownership/versioning, preferably a separate database. Downgrade and export/import compatibility require explicit tests before shipping paid state.

## Licensing

The public code remains AGPL. A private repository is a development boundary, not an exemption from AGPL obligations. Before distributing a closed-source derivative, verify copyright ownership, contributor permissions and third-party licenses, and establish a compatible commercial/dual-license arrangement if appropriate. This scaffold does not relicense existing code or promise that linking private code automatically permits proprietary distribution.

## Verification

Run `npm test`, `npm run build`, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, and the Rust library tests/checks for common changes. Pro adds its own boundary/build checks. Native window behavior, data isolation, file workflows and keyboard interactions still require desktop acceptance using `npm run tauri:dev`; never install development builds into `/Applications`.
