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
