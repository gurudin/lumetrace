# LumeTrace Migration Boundary

LumeTrace treats the migrated File Space as the application root. It does not copy Virelume's Task, Cell, Continuum, Blueprint, Agent CLI, or MemOS runtime.

## Retained states

- Empty and unconfigured storage
- Typical folder and file collections
- Maximum-density file grids and long names
- Import progress, cancellation, errors, and retries
- Multiple versions and historical preview
- Chinese and English
- Light and dark appearance
- Wide and narrow desktop windows
- Keyboard focus and Escape dismissal for overlays

## First migration milestone

The first milestone preserves the existing file-management behavior in a standalone desktop application. Stable cross-path identity, NAS repair, AI comparison, and citation are product directions for later milestones, not completion claims for this extraction.

## Initialization boundary

The generic existing-folder importer reads physical directories and files only. It preserves the directory hierarchy and creates a v1 snapshot without moving or deleting the originals. It does not inspect or migrate Virelume SQLite data. Product-specific importers for Eagle and similar tools belong to later adapter milestones.
