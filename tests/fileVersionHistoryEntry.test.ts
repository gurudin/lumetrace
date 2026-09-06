import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const entry = readFileSync(new URL("../src/pages/file-space/VersionHistoryBadge.tsx", import.meta.url), "utf8");
const openTimeline = page.slice(page.indexOf("const openTimeline = async"), page.indexOf("const selectTimelineVersion = async"));

test("the history badge is a sibling button, preserving first-child file-card contracts", () => {
  assert.match(page, /<\/button>\s*\{fileLayoutMode !== "list" && shouldShowFileVersionBadge\(file.versionCount\) \? \(\s*<VersionHistoryBadge/);
  assert.match(entry, /return <>\s*<button ref=\{badge\} className="file-space-file-version-count is-history-action"/);
  assert.match(page, /<FileArtwork file=\{file\} onImageDimensions=\{recordImageDimensions\} showVersionBadge=\{false\} \/>/);
  assert.match(page, /onPointerDown=\{\(event\) => beginFileDragGesture\(event, file.id\)\}/);
  assert.match(page, /onClick=\{\(event\) => handleFileClick\(event, file.id\)\}/);
  for (const format of ["Markdown", "Text", "Pdf", "Image"]) {
    const preview = readFileSync(new URL(`../src/pages/file-space/${format}PreviewOverlay.tsx`, import.meta.url), "utf8");
    assert.match(preview, /\.file-space-file-card > button:first-child/);
  }
});

test("badge click opens the existing timeline without triggering preview, drag, or Space shortcuts", () => {
  assert.match(entry, /onPointerDown=\{\(event\) => event.stopPropagation\(\)\}/);
  assert.match(entry, /onDoubleClick=\{\(event\) => event.stopPropagation\(\)\}/);
  assert.match(entry, /\["Enter", " ", "ArrowDown", "Tab"\].includes\(event.key\)\) event.stopPropagation/);
  assert.match(entry, /if \(event.detail > 1\) return/);
  assert.match(page, /await openTimeline\(file, versionId\)/);
  assert.match(page, /onContextMenu=\{\(event\) => openFileContextMenu\(event, file.id\)\}/);
  assert.doesNotMatch(entry, /beginFileDragGesture|handleFileClick|new MouseEvent/);
});

test("opening history preserves paging, selection and other selected files' inspector data", () => {
  assert.match(openTimeline, /"get_task_file_timeline", \{ fileId: file.id \}/);
  assert.doesNotMatch(openTimeline, /get_file_space_snapshot|setSnapshot|updateSelectedFiles/);
  assert.match(openTimeline, /selectedFileIdsRef.current.size === 1 && selectedFileIdsRef.current.has\(file.id\)/);
  assert.match(openTimeline, /defaultVersionComparison\(loaded.versions, selectedId\)/);
  assert.match(openTimeline, /setTimelinePanelOpen\(true\)/);
});

test("the entry has per-file loading feedback, keyboard focus, and reduced motion support", () => {
  assert.match(entry, /aria-busy=\{busy\} disabled=\{disabled\}/);
  assert.match(page, /busy=\{openingTimelineFileId === file.id\}/);
  assert.match(entry, /aria-label=\{[^\n]*name/);
  assert.match(page, /timelineBadgeReturnFocusRef.current = anchor/);
  assert.match(page, /trigger.focus\(\{ preventScroll: true \}\)/);
  // A full-window dialog no longer has a clickable outside region.
  assert.match(page, /window\.addEventListener\("focusin", keepTimelineFocus\)/);
  assert.doesNotMatch(page, /const closeOnOutsidePointerDown =/);
  assert.match(css, /\.file-space-file-version-count.is-history-action:focus-visible\s*\{[^}]*outline:/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.file-space-file-version-count.is-history-action \.is-spinning \{ animation: none/);
});
