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
