# LumeTrace Agent Instructions

## Product boundary

LumeTrace is a local-first, traceable file workspace. Preserve physical-file ownership, file-version history, task/source traceability, search, preview, and recovery when changing the product. Do not present planned AI, sharing, NAS collaboration, permissions, or cloud features as working capabilities before their real data and execution paths exist.

## Apple UI and interaction gate

Before planning, implementing, or reviewing any change that affects UI or page interaction, check that the proposed behavior and rendered result follow the current Apple Human Interface Guidelines and current macOS interaction and visual conventions. This check is mandatory for every affected screen, control, popover, panel, sheet, menu, selection state, drag interaction, keyboard interaction, light appearance, and dark appearance.

Use Apple's current official guidance as the primary source:

- <https://developer.apple.com/design/human-interface-guidelines/designing-for-macos>
- <https://developer.apple.com/design/human-interface-guidelines/sidebars>
- <https://developer.apple.com/design/human-interface-guidelines/toolbars>
- <https://developer.apple.com/design/human-interface-guidelines/popovers>
- <https://developer.apple.com/design/human-interface-guidelines/sheets>

For every material UI or interaction change:

1. Define the information hierarchy, selection behavior, scroll ownership, disclosure behavior, focus and keyboard behavior, and light/dark state before implementation.
2. Prefer familiar macOS patterns and system semantics over decorative imitation. Use a Popover for brief contextual choices, a non-modal Panel for repeated work while the parent remains usable, and a Sheet only for a short task with an explicit end state.
3. Keep parent content undimmed for Popovers and non-modal Panels. Dim and block the parent only for a modal Sheet or alert.
4. Use system font stacks, semantic/dynamic colors, restrained materials, native-sized controls, visible focus, and the user's accent where applicable. Dark appearance must be designed with semantic surfaces, not produced by simple color inversion or pure black.
5. Do not add inert toolbar controls or fabricate data to match a mockup. Every enabled control must perform a real action; unavailable capability must be omitted, disabled, or explicitly described as unavailable.
6. Preserve accessibility labels, keyboard access, Escape behavior, reduced-motion behavior, and adequate hit targets.
7. Verify with realistic maximum-density content at the default window size and minimum supported size. Check light and dark appearances, scrolling, selection, menus, Popovers, Panels, Sheets, empty states, loading states, errors, and disabled states.
8. Compare the rendered result with the approved reference at the level of hierarchy, grouping, spacing, disclosure, and control placement, not only colors and corner radii.

## Product editions and repository boundary

- Maintain only two repositories: public `lumetrace` for Community and private `lumetrace-pro` for the Official/Commercial product. The commercial repository contains both Pro and Team capabilities; do not create a third codebase.
- Keep the product hierarchy `Free ⊂ Pro ⊂ Team`. Free equals Community. Pro equals Free plus personal professional capabilities. Team equals Free plus all Pro capabilities plus collaboration. A Team license must always grant Pro capabilities.
- Community is a complete, usable local-first personal product, not a trial or crippled demo. All currently released Community capabilities remain free, including workspaces, file management, previews, trash, backup/restore, file-name and full-text search, optional semantic/vector search, SQLite indexing, AI providers and file Q&A, automatic history, Timeline, Diff and restore.
- Community receives bug fixes, security fixes, performance and compatibility improvements, UI/UX polish, and work required for existing capabilities to function correctly. Never remove or gate an existing Community capability to manufacture Pro value.
- Pro is for new advanced single-user workflows, such as professional file processing, advanced Diff, automation, batch workflows, saved searches, deep integrations and other power-user features. These are candidates until the user explicitly approves them; do not invent Pro features merely to fill the tier.
- Team is for multi-user collaboration, shared workspaces, self-owned NAS/S3/OSS/object storage, connection and sync, permissions, roles, activity/audit logs and team administration. LumeTrace coordinates these capabilities but does not make official cloud file hosting the default model.
- Classify each requested change before implementation. Existing-feature fixes and improvements belong to Community and must sync to Commercial. New advanced personal workflows are Pro candidates. Multi-user, shared-storage, permission, audit and administration capabilities belong to Team.
- If ownership between Community, Pro or Team is not unambiguous, stop and ask the user before editing code. Never infer the paid boundary merely because a feature appears monetizable.
- Commercial licensing must resolve through `LicenseProvider -> LicenseTier -> Entitlements -> Capabilities`, not scattered `isPro` or `isTeam` checks. Support future website/CDK, Mac App Store and team-license providers through this abstraction.
- Shared frontend initialization is `src/bootstrap.tsx`; shared build configuration is `build/createViteConfig.ts`. The Rust library takes its caller's Tauri context; each edition owns its binary/configuration. Never duplicate the common command handler, worker lifecycle or migrations in Pro.
- See `docs/EDITIONS.md` for repository boundaries, dependency updates, data isolation and licensing constraints. Do not publish private modules or local competitor research into this repository.

## Desktop development

- During development, run the desktop application with `npm run tauri:dev`.
- Do not install development builds into `/Applications`.
- Use the user-facing product name `LumeTrace` without a space; keep internal package, database, bundle, protocol, and storage identifiers as `lumetrace` unless a migration is explicitly planned.
- Preserve the existing file-card DOM contracts used by previews, system opening, and drag behavior unless every dependent path is updated and verified together.

## Git repository and delivery rules

- The GitHub repository is `git@github.com:gurudin/lumetrace.git`, configured as `origin`.
- `main` is the default branch. `develop` is the development branch and the only branch on which agents may create commits.
- Stay on `develop` throughout development. Before editing, committing, or pushing, verify the current branch. If it is not `develop`, stop and ask the user rather than changing branches automatically.
- Do not create, switch to, merge, rebase, reset, delete, or push another branch without an explicit user instruction. In particular, never merge or push development changes into `main` automatically.
- After each completed change, review the diff, run the relevant checks, create a commit, and push it to `origin develop`. This is standing authorization for that commit and push; do not request confirmation each time.
- All commit titles and descriptions must be written in English. Prefer concise conventional commit messages, such as `fix: refresh indexes after file edits`.
- Commit only the intended project changes. Exclude credentials, local user files, generated artifacts, and unrelated work. Never force-push or rewrite published history without explicit authorization.
- If verification, commit, or push fails, report the exact failure and do not claim that delivery is complete.
- 每次提交完成后，最终回复末尾必须单独输出固定格式：提交：社区版 `<本次实际提交短哈希>`，专业版 `<本次实际提交短哈希>`。仅提交社区版时只写“提交：社区版 `<本次实际提交短哈希>`”，仅提交专业版时只写“提交：专业版 `<本次实际提交短哈希>`”；不得列出本次未提交的版本或沿用历史哈希，推送失败须另外明确说明。

## Mandatory Community-to-Pro synchronization

- Pro includes every Community capability. Every completed Community change must also be delivered to Pro; a Community-only push is not completion.
- Test, commit and push the Community change on `develop`, then update Pro’s pinned `vendor/lumetrace` revision to that verified commit. Run Pro integration checks, commit the dependency update and push Pro `develop`.
- Never copy shared implementation into Pro or edit tracked submodule files. Preserve unrelated local work. If synchronization or verification is blocked, report it explicitly and do not claim both editions are complete.
