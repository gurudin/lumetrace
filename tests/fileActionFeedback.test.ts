import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(`../src/${path}`, import.meta.url), "utf8");

test("system opening remains mounted for capable external workspaces", () => {
  const app = read("App.tsx");
  assert.match(app, /capabilities\?\.content !== false \? <ExternalDocumentOpenBridge/);
  assert.doesNotMatch(app, /!workspace\?\.active \? <ExternalDocumentOpenBridge/);
  const bridge = read("pages/file-space/ExternalDocumentOpenBridge.tsx");
  assert.match(bridge, /if \(pendingFileRef.current === target.fileId\) return/);
  assert.match(bridge, /finally \{\s+if \(pendingFileRef.current === target.fileId\)/);
  assert.match(bridge, /\[closeNotice, copy.desktopOnly, invoke\]/);
  assert.match(bridge, /listen<ExternalDocumentEditEvent>\("file-space-external-edit-state"/);
  assert.match(bridge, /payload.state === "saved"/);
  assert.match(bridge, /payload.reason === "conflict"/);
});

test("notification feedback covers lookup and history read with a synchronous duplicate guard", () => {
  const page = read("pages/file-space/FileSpacePage.tsx");
  const action = page.slice(page.indexOf("const viewVersionNotification ="), page.indexOf("useEffect(() => {\n    if (!importConflictFeedback"));
  assert.match(action, /if \(!notification \|\| openingNotificationRef.current\) return/);
  assert.ok(action.indexOf("setOpeningNotificationId(notification.versionId)") < action.indexOf("await invoke"));
  assert.match(action, /finally \{\s+openingNotificationRef.current = null/);
  assert.match(page, /aria-busy=\{openingNotificationId === versionNotification.versionId\}/);
  assert.match(page, /disabled=\{openingNotificationId !== null\}/);
});
