import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
const compiled = ts.transpileModule(readFileSync(new URL("../src/shared/extensions/workspaceCommands.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
function harness() {
  const calls: string[] = [], exports: any = {};
  new Function("require", "exports", compiled)(() => ({ invoke: async (command: string) => { calls.push(command); return "local"; } }), exports);
  return { ...exports, calls };
}
test("default source stays native; external commands cannot fall through to local", async () => {
  const api = harness();
  assert.equal(await api.workspaceInvoke("get_file_space_snapshot"), "local");
  const remote = { key: "remote", invoke: async () => { throw new Error("unsupported"); } };
  api.setWorkspaceCommandSource(remote);
  await assert.rejects(api.workspaceInvoke("delete_file_space_file"), /unsupported/);
  assert.deepEqual(api.calls, ["get_file_space_snapshot"]);
  await api.workspaceInvoke("get_file_space_workspaces");
  assert.equal(api.calls.at(-1), "get_file_space_workspaces");
});
test("stale replies and delayed callbacks are rejected across source switches", async () => {
  const api = harness(); let finish!: (v: string) => void;
  const remote = { key: "remote", invoke: () => new Promise(resolve => { finish = resolve; }) };
  api.setWorkspaceCommandSource(remote);
  const old = api.bindWorkspaceInvoke(remote), pending = old("list_file_space_files");
  api.setWorkspaceCommandSource(null); finish("remote data");
  await assert.rejects(pending, /workspace changed/);
  await assert.rejects(old("create_file_space_text_file"), /workspace changed/);
  assert.deepEqual(api.calls, []);
  const unmounted = api.bindWorkspaceInvoke(null, () => false);
  await assert.rejects(unmounted("create_file_space_text_file"), /workspace changed/);
});

test("visible external actions remain source-bound instead of operating on personal data", async () => {
  const api = harness(), requests: string[] = [];
  api.setWorkspaceCommandSource({ key: "external", invoke: async (command: string) => { requests.push(command); throw new Error("not connected"); } });
  const actions = ["search_file_space_files", "get_file_space_search_preview", "ask_file_space_ai", "get_task_file_timeline", "get_semantic_search_status", "install_semantic_search_model", "export_file_space_backup", "restore_file_space_backup", "create_file_space_text_file", "delete_file_space_file"];
  for (const action of actions) await assert.rejects(api.workspaceInvoke(action), /not connected/);
  assert.deepEqual(requests, actions);
  assert.deepEqual(api.calls, []);
  await api.workspaceInvoke("get_ai_service_settings");
  assert.deepEqual(api.calls, ["get_ai_service_settings"]);
});

test("archive restrictions block all whole-space backup commands but preserve file recovery and other actions", async () => {
  const api = harness(), requests: string[] = [];
  const archives = ["export_file_space_backup", "inspect_file_space_backup", "restore_file_space_backup"];
  api.setWorkspaceCommandSource({ key: "external", capabilities: { backup: false }, invoke: async (command: string) => { requests.push(command); return "external"; } });
  for (const command of archives) await assert.rejects(api.workspaceInvoke(command), /backup and restore are disabled/);
  assert.deepEqual(requests, []);
  assert.deepEqual(api.calls, []);
  for (const command of ["set_current_task_file_version", "get_task_file_timeline", "search_file_space_files", "ask_file_space_ai", "get_semantic_search_status", "create_file_space_text_file"]) {
    assert.equal(await api.workspaceInvoke(command), "external");
  }
  assert.equal(requests.length, 6);
  assert.deepEqual(api.calls, []);
  api.setWorkspaceCommandSource(null);
  for (const command of archives) assert.equal(await api.workspaceInvoke(command), "local");
  assert.deepEqual(api.calls, archives);
});
