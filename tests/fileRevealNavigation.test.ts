import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { FileRevealNavigation } from "../src/pages/file-space/fileRevealNavigation.ts";
import { pruneSelection, selectionVisibilityWithPendingReveal } from "../src/pages/file-space/fileSelection.ts";
import { calculateFileListLayout, calculateJustifiedFileLayout, visibleJustifiedFileIds } from "../src/pages/file-space/fileJustifiedLayout.ts";
import { resolveFileDoubleClickRoute } from "../src/pages/file-space/fileOpenRouting.ts";
import {
  attachFileOpenSearchContext,
  fileOpenSearchContextFromEvent,
  type FileOpenSearchContext,
} from "../src/pages/file-space/fileOpenSearchContext.ts";

const file = { id: "test-file", folderId: "documents", name: "test-version.md" };
const other = { id: "other-file", folderId: "elsewhere", name: "other.txt" };

test("global search opens the selected result after revealing its card", () => {
  const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");
  assert.match(
    page,
    /revealFileInWorkspace\(file, true, context \?\? undefined\);/,
  );
});

test("global search reopens empty and shows the split preview only for body matches", () => {
  const panel = readFileSync(new URL("../src/pages/file-space/FileSpaceSearchPanel.tsx", import.meta.url), "utf8");
  const styles = readFileSync(new URL("../src/pages/file-space/file-space-search-panel.css", import.meta.url), "utf8");
  assert.match(panel, /useEffect\(\(\) => \{\s*if \(open && !wasOpenRef\.current\) \{\s*returnFocusRef\.current/);
  assert.match(panel, /const showContentPreview = activeMatch\?\.contentMatch === true;/);
  assert.match(panel, /file-space-global-search-panel\$\{showContentPreview \? " has-content-preview" : ""\}/);
  assert.match(panel, /showContentPreview \? <section className="file-space-global-search-preview"/);
  assert.match(styles, /\.file-space-global-search-panel \{[\s\S]*?width: min\(660px,/);
  assert.match(styles, /\.file-space-global-search-panel\.has-content-preview \{[\s\S]*?width: min\(1120px,/);
  assert.match(styles, /width var\(--motion-duration-disclose\) var\(--motion-ease-disclose\)/);
});

test("search context reaches only the matching file open event", () => {
  const context: FileOpenSearchContext = {
    fileId: file.id,
    query: "version restore",
    lineNumber: 18,
    pageNumber: null,
    matchIndex: 1,
  };
  const event = attachFileOpenSearchContext(new Event("dblclick"), context);
  assert.equal(fileOpenSearchContextFromEvent(event, file.id), context);
  assert.equal(fileOpenSearchContextFromEvent(event, other.id), null);
  assert.equal(fileOpenSearchContextFromEvent(new Event("dblclick"), file.id), null);
});

test("reveal navigation carries search context into the one open action", () => {
  const context: FileOpenSearchContext = {
    fileId: file.id,
    query: "version",
    lineNumber: 4,
    pageNumber: null,
    matchIndex: 0,
  };
  const navigation = new FileRevealNavigation<typeof file, FileOpenSearchContext>();
  const request = navigation.start(file, true, true, context);
  let received: FileOpenSearchContext | undefined;
  assert.equal(navigation.completeWithTarget(request, file, () => {}, (_target, value) => {
    received = value;
  }), true);
  assert.equal(received, context);
});

test("search-open context is consumed by every searchable document preview", () => {
  for (const name of ["MarkdownPreviewOverlay", "TextPreviewOverlay", "PdfPreviewOverlay"]) {
    const source = readFileSync(new URL(`../src/pages/file-space/${name}.tsx`, import.meta.url), "utf8");
    assert.match(source, /fileOpenSearchContextFromEvent\(event,/);
    assert.match(source, /SearchResult|highlightSearchResult/);
  }
});

test("a single citation click survives the old page and selects without opening", () => {
  const navigation = new FileRevealNavigation<typeof file>();
  const request = navigation.start(file);
  const actions: string[] = [];
  const focus = () => actions.push("select");
  const open = () => actions.push("open");

  // The old page can already contain the target (e.g. All Files or same folder).
  assert.equal(navigation.readyForLayout(file.folderId), null);
  assert.equal(navigation.completeWithTarget(request, file, focus, open), false);
  assert.equal(navigation.pending, request);
  assert.deepEqual(actions, []);

  const page = navigation.acceptPage(request, file.folderId, [file]);
  assert.equal(page.length, 1);
  assert.equal(navigation.readyForLayout(file.folderId), request);
  assert.equal(navigation.completeWithTarget(request, file, focus, open), true);
  assert.deepEqual(actions, ["select"]);
  assert.equal(navigation.pending, null);
});

test("cross-folder navigation pins a target outside the first bounded page", () => {
  const navigation = new FileRevealNavigation<typeof file>();
  const request = navigation.start(file);
  const selection = new Set([file.id]);
  const oldVisible = selectionVisibilityWithPendingReveal([other.id], navigation.pending?.file.id);
  assert.deepEqual([...pruneSelection(selection, oldVisible)], [file.id]);
  assert.equal(navigation.readyForLayout(other.folderId), null);
  const firstPage = Array.from({ length: 200 }, (_, index) => ({ ...file, id: `page-${index}` }));
  const revealedPage = navigation.acceptPage(request, file.folderId, firstPage);
  assert.equal(revealedPage.length, 201);
  assert.equal(revealedPage[0], file);
  assert.deepEqual([...pruneSelection(selection, new Set(revealedPage.map((item) => item.id)))], [file.id]);
});

test("a double click replaces a pending single click and ignores its late page", () => {
  const navigation = new FileRevealNavigation<typeof file>();
  const single = navigation.start(file);
  const double = navigation.start(file, true);
  navigation.acceptPage(single, file.folderId, [file]);
  assert.equal(double.pageReady, false);
  assert.equal(navigation.readyForLayout(file.folderId), null);
  navigation.acceptPage(double, file.folderId, [file]);
  const actions: string[] = [];
  const focus = () => actions.push("select");
  const open = () => actions.push("open");
  assert.equal(navigation.completeWithTarget(single, file, focus, open), false);
  assert.equal(navigation.completeWithTarget(double, file, focus, open), true);
  assert.equal(navigation.completeWithTarget(double, file, focus, open), false);
  assert.deepEqual(actions, ["select", "open"]);
});

for (const mode of ["list", "adaptive"] as const) {
  test(`${mode} opens a virtualized citation once, even when its card needs more than two frames`, () => {
    const navigation = new FileRevealNavigation<typeof file>();
    const request = navigation.start(file, true);
    const page = Array.from({ length: 199 }, (_, index) => ({ ...file, id: `file-${index}` })).concat(file);
    navigation.acceptPage(request, file.folderId, page);
    const items = page.map(({ id }) => ({ id, aspectRatio: 1.5 }));
    const layout = mode === "list"
      ? calculateFileListLayout({ containerWidth: 900, rowHeight: 48, verticalGap: 4, previewSize: 36, items })
      : calculateJustifiedFileLayout({ containerWidth: 900, targetPreviewHeight: 120,
          horizontalGap: 12, verticalGap: 20, previewDetailsGap: 8, detailsHeight: 32, items });
    const initialIds = visibleJustifiedFileIds(page.map(({ id }) => id), layout, 0, 600);
    assert.equal(initialIds.includes(file.id), false);
    const actions: string[] = [];
    const focus = () => actions.push("select");
    const open = () => actions.push(resolveFileDoubleClickRoute(file.name));
    for (let frame = 0; frame < 5; frame += 1) {
      assert.equal(navigation.completeWithTarget(request, null, focus, open), false);
      assert.equal(navigation.pending, request);
    }
    const top = layout.placements[file.id].y;
    const renderedIds = visibleJustifiedFileIds(page.map(({ id }) => id), layout, top - 300, top + 300);
    assert.ok(renderedIds.includes(file.id));
    assert.ok(renderedIds.length < page.length);
    assert.equal(navigation.completeWithTarget(request, file, focus, open), true);
    assert.deepEqual(actions, ["select", "internal-preview"]);
    assert.equal(navigation.completeWithTarget(request, file, focus, open), false);
  });
}

test("new references, folder changes and workspace changes invalidate earlier reveals", () => {
  const navigation = new FileRevealNavigation<typeof file>();
  const old = navigation.start(file, true);
  const next = navigation.start(other);
  const oldPage = [file];
  assert.equal(navigation.acceptPage(old, file.folderId, oldPage), oldPage);
  assert.equal(next.pageReady, false);
  navigation.acceptPage(next, file.folderId, oldPage);
  assert.equal(next.pageReady, false);
  navigation.acceptPage(next, other.folderId, [other]);
  navigation.cancel();
  assert.equal(navigation.readyForLayout(other.folderId), null);
  assert.equal(navigation.completeWithTarget(next, other, () => assert.fail("stale focus"), () => assert.fail("stale open")), false);
});

test("root-level and browser fixture references do not require a folder id or historical version id", () => {
  const navigation = new FileRevealNavigation<{ id: string; folderId: string | null; name: string }>();
  const rootFile = { ...file, folderId: null };
  const request = navigation.start(rootFile, true, true);
  assert.equal(navigation.readyForLayout(null), request);
  assert.equal(navigation.readyForLayout(file.folderId), null);
  assert.equal(navigation.completeWithTarget(request, rootFile, () => {}, () => {}), true);
});
