import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { versionSummaryPosition } from "../src/pages/file-space/versionSummaryPosition.ts";

test("version summary flips above bottom-edge badges and stays inside narrow windows", () => {
  for (const viewport of [{ width: 1280, height: 800 }, { width: 640, height: 640 }, { width: 320, height: 480 }]) {
    for (const left of [8, viewport.width - 72]) {
      for (const top of [80, viewport.height - 40]) {
        const anchor = { left, right: left + 60, top, bottom: top + 20 };
        const position = versionSummaryPosition(anchor, viewport, 340);
        assert.ok(position.left >= 12);
        assert.ok(position.left + position.width <= viewport.width - 12);
        assert.ok(position.top >= 12);
        assert.ok(position.top + Math.min(340, position.maxHeight) <= viewport.height - 12);
        assert.equal(position.side, top === 80 ? "below" : "above");
        assert.ok(position.arrow >= 16 && position.arrow <= position.width - 16);
      }
    }
  }
});

test("summary requests metadata only, has stale-response cleanup, and is nonmodal", () => {
  const source = (name: string) => readFileSync(new URL(`../src/pages/file-space/${name}`, import.meta.url), "utf8");
  const badge = source("VersionHistoryBadge.tsx");
  assert.match(badge, /aria-modal="false"/);
  assert.match(badge, /setTimeout\(\(\) => show\(\), 350\)/);
  assert.match(badge, /active = false/);
  assert.match(badge, /createPortal/);
  assert.match(badge, /useVersionAnnotationUpdates\(setSummary\)/);
  const page = source("FileSpacePage.tsx");
  const loader = page.slice(page.indexOf("loadSummary={async"), page.indexOf("onContextMenu=", page.indexOf("loadSummary={async")));
  assert.match(loader, /get_file_version_summary/);
  assert.doesNotMatch(loader, /get_task_file_timeline|read_task_file_version/);
  assert.match(loader, /summary.workspaceId !== workspaceId \|\| summary.fileId !== file.id/);
});
