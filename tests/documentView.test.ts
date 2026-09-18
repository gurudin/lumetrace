import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
const read = (file: string) => readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
test("optional document adapter preserves built-in content and host-owned saving", () => {
  const overlay = read("src/pages/file-space/MarkdownPreviewOverlay.tsx");
  assert.match(overlay, /DocumentViewSlot/);
  assert.match(overlay, /fallback=\{contentView\}/);
  assert.match(overlay, /historicalVersionSelected \|\| saving \|\| !open/);
  assert.match(overlay, /save_file_space_markdown/);
  assert.match(overlay, /useSearchResultHighlight/);
  assert.match(overlay, /PreviewVersionDiff/);
  assert.doesNotMatch(overlay, /LumeMark|pluginRuntime|isPro/);
});
test("content adapters receive drafts, never native commands or save authority", () => {
  const contract = read("src/shared/extensions/DocumentView.tsx");
  assert.match(contract, /onChange: \(value: string\) => void/);
  assert.match(contract, /getDerivedStateFromError/);
  assert.doesNotMatch(contract, /invoke|onSave|writeFile|fetch\(/);
});
test("the real view slot renders the current draft and retains fallback after failure", () => {
  const exports: Record<string, any> = {};
  const compiled = ts.transpileModule(read("src/shared/extensions/DocumentView.tsx"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  new Function("require", "exports", compiled)(createRequire(import.meta.url), exports);
  const props = { sessionKey: "space:file:2", value: "latest draft", readOnly: true,
    fallback: createElement("textarea", { defaultValue: "latest draft" }),
    view: (p: any) => createElement("article", { "data-readonly": p.readOnly }, p.value) };
  assert.match(renderToStaticMarkup(createElement(exports.DocumentViewSlot, props)), /data-readonly="true">latest draft/);
  const failed = new exports.DocumentViewSlot(props);
  failed.state = exports.DocumentViewSlot.getDerivedStateFromError();
  assert.equal(renderToStaticMarkup(failed.render()), '<textarea>latest draft</textarea>');
});
