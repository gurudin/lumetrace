# Shared offline Help Center

The bottom-right question-mark button opens a non-modal guide without dimming or disabling the file workspace. Its tooltip and accessible label identify Help Center. The shared App mounts it in both editions, including local setup; edition management pages retain their own surface.

The guide includes 15 topics: getting started, workspaces, import, organization and tags, preview/editing, search, cloud AI, local AI, Agent CLIs, Lumie, semantic indexing, history/Diff/restore, Trash/backups, background troubleshooting, and appearance/shortcuts. Complete articles are bundled in Simplified Chinese and English. Controls support all eight application locales; other locales explicitly disclose the English article fallback.

Search matches titles and instructions with case-insensitive, all-word matching and prioritizes title matches. The topic list and article scroll independently. Closing preserves reading state for this application session. Opening focuses search; Escape from the guide closes it and restores the trigger. Tab is not trapped. Workspace search shortcuts ignore help targets. No application commands, cloud requests, telemetry or credential reads are performed by the guide.

## Design and acceptance

Reviewed Apple’s official [Offering help](https://developer.apple.com/design/human-interface-guidelines/offering-help) and [Panels](https://developer.apple.com/design/human-interface-guidelines/panels) guidance. The task-oriented guide uses existing semantic surfaces and accent colors, clear selection, independent scrolling, visible keyboard focus, and an explicit close action. There is no modal backdrop or animation. Actual modal workflows remain above the guide and hide its controls.

On 2026-09-11, synthetic browser previews were inspected in dark and light appearance, at 1280-pixel width and the 920 × 640 minimum window. Chinese and English text, topic filtering, title ranking, empty results, focus return, Escape and Cmd+K isolation were checked. The reader had no horizontal overflow at minimum size. No real workspace or AI provider was used. Native Finder drag/drop remains outside this browser acceptance; the guide does not change those operations.

Community validation: 194 Node tests passed, production build and Rust format/check passed; Rust library tests passed 190 with 2 ignored. Existing bundle-size warnings remain. Pro must pin this verified shared revision and run its own integration checks before delivery is complete.
