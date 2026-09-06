import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createInstance } from "i18next";
import { shouldShowVersionTimelineByDefault } from "../src/pages/file-space/versionTimelineVisibility.ts";
import { en } from "../src/shared/i18n/locales/en.ts";
import { zh } from "../src/shared/i18n/locales/zh.ts";

const source = (name: string) => readFileSync(new URL(`../src/pages/file-space/${name}`, import.meta.url), "utf8");

test("preview history starts collapsed regardless of the number of versions", () => {
  assert.equal(shouldShowVersionTimelineByDefault(), false);
});

test("all detail previews reset disclosure on open and show the total count from existing metadata", () => {
  for (const format of ["Markdown", "Text", "Pdf", "Image"]) {
    const overlay = source(`${format}PreviewOverlay.tsx`);
    assert.match(overlay, /shouldShowVersionTimelineByDefault\(\)/, format);
    assert.match(overlay, /<PreviewFileHeading name=\{request.name\} versionCount=\{timeline\?\.versions.length \?\? request.versionCount\}>/, format);
    assert.match(overlay, /<VersionTimelineToggle/, format);
    assert.match(overlay, /<small>\{copy.historical(?:Version)?\}<\/small>/, format);
  }
  const markdown = source("MarkdownPreviewOverlay.tsx");
  assert.match(markdown, /setTimelineVisible\(timelineInitiallyVisible\)/);
  assert.match(markdown, /if \(timelineInitiallyVisible\) void loadTaskTimeline\(card.request\)/);
  assert.match(markdown, /if \(nextVisible && !timeline && !timelineLoading\) void loadTaskTimeline\(request\)/);
  assert.match(markdown, /if \(!timeline && !timelineLoading\) void loadTaskTimeline\(request\)/);
});

test("version summary handles zero, singular, plural and updated counts without changing sidebar copy", async () => {
  const i18n = createInstance();
  await i18n.init({ lng: "zh", resources: { en: { translation: en }, zh: { translation: zh } } });
  for (const count of [0, 1, 2, 40]) {
    assert.equal(i18n.t("fileSpace.preview.common.fileVersionCount", { count }), `当前文件共 ${count} 个版本`);
  }
  await i18n.changeLanguage("en");
  assert.equal(i18n.t("fileSpace.preview.common.fileVersionCount", { count: 1 }), "This file has 1 version");
  assert.equal(i18n.t("fileSpace.preview.common.fileVersionCount", { count: 2 }), "This file has 2 versions");
  assert.equal(i18n.t("fileSpace.preview.common.versionCount", { count: 2 }), "2 versions");
});

test("summary sits below the filename, remains muted and does not replace editing status", () => {
  const heading = source("PreviewFileHeading.tsx");
  assert.match(heading, /<strong title=\{name\}>\{name\}<\/strong>\s*\{children\}/);
  assert.match(heading, /<\/div>\s*<span className="file-preview-heading-summary"/);
  const css = source("preview-file-heading.css");
  assert.match(css, /\.file-preview-heading-summary\s*\{[^}]*color: var\(--preview-heading-secondary, var\(--text-muted\)\)/);
  assert.match(css, /\.file-preview-heading-summary\s*\{[^}]*font-size: var\(--font-size-caption\)/);
  assert.match(css, /\.file-preview-heading-row > strong\s*\{[^}]*text-overflow: ellipsis/);
  assert.doesNotMatch(css, /\.file-preview-heading-summary\s*\{[^}]*display: none/);
  const markdown = source("MarkdownPreviewOverlay.tsx");
  assert.match(markdown, /<small>\{copy.unsaved\}<\/small>/);
  assert.match(markdown, /<small>\{copy.saved\}<\/small>/);
});
