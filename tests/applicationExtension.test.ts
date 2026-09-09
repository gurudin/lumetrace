import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const require = createRequire(import.meta.url);
const source = readFileSync(new URL("../src/shared/extensions/ApplicationExtension.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(source.replace('import "./application-extension.css";', ""), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
}).outputText;
const module = { exports: {} as Record<string, any> };
new Function("require", "exports", compiled)(require, module.exports);
const { ApplicationExtensionHost, ApplicationExtensionEntry } = module.exports;
const { WorkspaceExtensionContext, useApplicationExtension } = module.exports;
const workspace = createElement("div", { "data-workspace": "preserved" }, "Existing workspace");

test("community without an extension renders its existing workspace unchanged", () => {
  assert.equal(renderToStaticMarkup(createElement(ApplicationExtensionHost, null, workspace)), renderToStaticMarkup(workspace));
  assert.equal(renderToStaticMarkup(createElement(ApplicationExtensionEntry, { placement: "setup" })), "");
});

test("extension pages retain the mounted workspace and hide it from interaction", () => {
  const extension = {
    initiallyOpen: true,
    Entry: () => createElement("button", null, "Custom entry"),
    Page: ({ active }: { active: boolean }) => createElement("section", { "data-active": active }, "Extension page"),
  };
  const html = renderToStaticMarkup(createElement(ApplicationExtensionHost, { extension }, workspace));
  assert.match(html, /application-workspace-surface[^>]*hidden=""[^>]*inert=""/);
  assert.match(html, /data-workspace="preserved"/);
  assert.match(html, /data-active="true"/);
  assert.doesNotMatch(source, /license|cdk|lumetrace-pro|team_code/i);
});

test("setup and settings slots use one extension, without imposing a local workspace", () => {
  for (const placement of ["setup", "settings"]) {
    const extension = {
      Entry: (props: { placement: string }) => createElement("button", null, props.placement),
      Page: () => null,
    };
    const html = renderToStaticMarkup(createElement(ApplicationExtensionHost, { extension }, createElement(ApplicationExtensionEntry, { placement })));
    assert.match(html, new RegExp(`<button>${placement}</button>`));
  }
});

test("additional storage keeps the same workbench visible without opening management", () => {
  function Status() { const state = useApplicationExtension(); return createElement("span", { "data-hidden": state.active, "data-management": state.pageActive }); }
  const extension = { Entry: () => null, Page: () => null, WorkspaceProvider: ({ children }: any) => createElement(WorkspaceExtensionContext.Provider, {
    value: { active: true, selectionKey: "external-one", source: {} },
  }, children) };
  const html = renderToStaticMarkup(createElement(ApplicationExtensionHost, { extension }, createElement("div", null, workspace, createElement(Status))));
  assert.doesNotMatch(html, /application-workspace-surface[^>]*hidden=""/);
  assert.match(html, /data-workspace="preserved"/);
  assert.match(html, /data-hidden="false" data-management="false"/);
  assert.doesNotMatch(html, /Alternate files/);
});

test("the body-portaled AI panel is hidden without cancelling work behind an extension page", () => {
  const ai = readFileSync(new URL("../src/pages/file-space/FileSpaceAiSurface.tsx", import.meta.url), "utf8");
  assert.match(ai, /workspaceHidden = useApplicationExtension\(\)\?\.active/);
  assert.match(ai, /hidden=\{workspaceHidden\}\s+inert=\{workspaceHidden\}/);
  assert.match(ai, /if \(!panelOpen \|\| workspaceHidden\) return undefined/);
  assert.doesNotMatch(ai, /workspaceHidden[^;]*stopQuestion\(/);
});
