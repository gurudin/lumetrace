import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

test("backup restoration retains the existing modal and synchronously guards repeat clicks", () => {
  const source = read("src/pages/file-space/FileSpaceSettingsMenu.tsx");
  const start = source.indexOf("const confirmRestoreBackup");
  const restore = source.slice(start, source.indexOf("  useEffect(() => {", start));
  assert.ok(restore.indexOf("restoreRunningRef.current = true") < restore.indexOf('"restore_file_space_backup"'));
  assert.match(restore, /finally\s*\{\s*restoreRunningRef.current = false/);
  assert.match(restore, /status: "restoring"/);
  assert.match(restore, /restoredBackupRef.current \?\? await invoke/);
  assert.match(source, /restoreFeedback.status === "restoring"/);
});

test("isolated restored workspaces are selected explicitly without publishing local files into the outgoing source", () => {
  const source = read("src/pages/file-space/FileSpaceSettingsMenu.tsx");
  assert.match(source, /if \(result.workspaceId\)[\s\S]*switch_file_space_workspace[\s\S]*onWorkspaceChanged\(mutation\)[\s\S]*onLocalSelect\(result.workspaceId\)/);
  assert.match(source, /if \(!mountedRef.current\) return;/);
  const native = read("src-tauri/src/workspace.rs");
  const restore = native.slice(native.indexOf("pub async fn restore_file_space_backup_as_new_workspace"), native.indexOf("pub fn rename_file_space_workspace"));
  assert.match(restore, /add_inactive/);
  assert.doesNotMatch(restore, /switch_database|switch_workspace|set_current/);
});
