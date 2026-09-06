# Optional application page entry

- Community has no registered extension and keeps its original first-run and settings UI.
- An edition can provide one entry component for both first-run and settings locations and one full-window page.
- The page uses shared appearance/localization; navigation preserves the mounted file workspace and background operations.
- Hidden surfaces are inert. Opening the page suppresses shared global search and external-drop actions. Returning restores focus to the originating control when it still exists.
- New code contains no team licensing, private endpoints, sample customer data or paid state.

Checks on 2026-09-07: `npm test` passed 169 tests, including server-rendered Community/extension boundary assertions; `npm run build` passed with the existing large-bundle warning. Professional activation and desktop interaction are verified separately by the consuming edition; these checks do not claim native GUI acceptance.
