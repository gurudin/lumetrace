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
