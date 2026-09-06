import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const diffCss = readFileSync(new URL("../src/pages/file-space/file-version-diff.css", import.meta.url), "utf8");
const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");

function rule(source: string, selector: string) {
  const start = source.indexOf(`${selector} {`);
  assert.ok(start >= 0, `Missing CSS rule: ${selector}`);
  return source.slice(start, source.indexOf("}", start));
}

test("timeline fills the app window and reserves the extra width for comparison", () => {
  assert.match(rule(css, ".file-space-timeline-backdrop"), /position: fixed/);
  assert.match(rule(css, ".file-space-timeline-backdrop"), /inset: 0/);
  const panel = rule(css, ".file-space-timeline-panel");
  assert.match(panel, /width: 100%/);
  assert.match(panel, /height: 100%/);
  assert.doesNotMatch(panel, /width: min\(/);
  assert.match(rule(css, ".file-space-timeline-body"), /grid-template-columns: clamp\(240px, 24vw, 320px\) minmax\(0, 1fr\)/);
  assert.match(rule(css, ".file-space-timeline-panel > header"), /padding-left: 64px/);
  assert.match(page, /className="file-space-timeline-drag-region" data-tauri-drag-region/);
});

test("full-window timeline owns keyboard focus while preserving portalled version menus", () => {
  assert.match(page, /className="file-space-timeline-panel" role="dialog" aria-modal="true"/);
  assert.match(page, /window\.addEventListener\("keydown", handleTimelineKeyDown\)/);
  assert.match(page, /window\.removeEventListener\("keydown", handleTimelineKeyDown\)/);
  assert.match(page, /window\.addEventListener\("focusin", keepTimelineFocus\)/);
  assert.match(page, /window\.removeEventListener\("focusin", keepTimelineFocus\)/);
  assert.match(page, /event\.target\.closest\("\.mac-select-menu"\)/);
  assert.match(page, /if \(expandedSelect\) \{\s*expandedSelect\.click\(\);\s*expandedSelect\.focus\(\{ preventScroll: true \}\);\s*return;/);
  assert.match(page, /trigger\.focus\(\{ preventScroll: true \}\)/);
});

test("timeline header stays outside the two bounded scroll panes", () => {
  const panel = rule(css, ".file-space-timeline-panel");
  assert.match(panel, /display: flex/);
  assert.match(panel, /flex-direction: column/);
  assert.match(panel, /overflow: hidden/);
  assert.match(rule(css, ".file-space-timeline-panel > header"), /flex: 0 0 auto/);

  const body = rule(css, ".file-space-timeline-body");
  assert.match(body, /flex: 1 1 0/);
  assert.match(body, /min-height: 0/);
  assert.match(body, /grid-template-rows: minmax\(0, 1fr\)/);
  assert.match(body, /overflow: hidden/);

  const panes = rule(css, ".file-space-timeline-body > section");
  assert.match(panes, /min-height: 0/);
  assert.match(panes, /overflow-y: auto/);
  assert.match(panes, /overscroll-behavior-y: contain/);
});

test("timeline content does not introduce nested vertical scroll traps", () => {
  const preview = rule(css, ".file-space-version-preview pre");
  assert.doesNotMatch(preview, /max-height:/);
  assert.match(preview, /overflow: visible/);
  assert.match(preview, /overflow-wrap: anywhere/);

  const scopedDiff = rule(css, ".file-space-version-preview .file-version-diff-table");
  assert.match(scopedDiff, /max-height: none/);
  assert.match(scopedDiff, /overscroll-behavior-y: auto/);
  const splitCode = rule(css, ".file-space-version-preview .file-version-diff-split-side > code");
  assert.match(splitCode, /min-width: 0/);
  assert.match(splitCode, /white-space: pre-wrap/);
  assert.match(splitCode, /overflow-wrap: anywhere/);
  // Other preview surfaces retain the shared Diff's bounded, horizontal-capable scroller.
  const sharedDiff = rule(diffCss, ".file-version-diff-table");
  assert.match(sharedDiff, /max-height: 52vh/);
  assert.match(sharedDiff, /overflow: auto/);
});

test("both scroll panes remain keyboard accessible and long titles retain their tooltip", () => {
  for (const className of ["file-space-version-list", "file-space-version-preview"]) {
    assert.match(page, new RegExp(`<section className="${className}" aria-label=\\{[^\\n]+\\} tabIndex=\\{0\\}>`));
  }
  assert.match(rule(css, ".file-space-timeline-body > section:focus-visible"), /outline:/);
  assert.match(page, /<h2 title=\{timelineFile\.name\}>\{timelineFile\.name\}<\/h2>/);
});

test("stacked narrow layout allocates bounded space to both panes", () => {
  assert.match(css, /@media \(max-width: 720px\)\s*\{\s*\.file-space-timeline-body\s*\{[^}]*grid-template-rows: minmax\(120px, \.4fr\) minmax\(0, 1fr\)/);
});
