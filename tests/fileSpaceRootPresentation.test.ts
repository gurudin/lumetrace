import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileSpaceRootView } from "../src/pages/file-space/fileSpaceRootPresentation.ts";

test("presents an initial load failure instead of the unconfigured setup screen", () => {
  assert.equal(fileSpaceRootView(false, "database unavailable", "unconfigured"), "loadError");
  assert.equal(fileSpaceRootView(false, null, "unconfigured"), "setup");
});

test("keeps loading and ready workspace states distinct", () => {
  assert.equal(fileSpaceRootView(true, "database unavailable", "unconfigured"), "loading");
  assert.equal(fileSpaceRootView(false, null, "ready"), "workspace");
});

test("successful snapshot delivery clears a stale initial error without reloading the workspace", () => {
  const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");
  const receiver = page.slice(page.indexOf("const applySavedMarkdownSnapshot"), page.indexOf('window.addEventListener("lumetrace:file-space-snapshot"'));
  assert.match(receiver, /if \(next\)\s*\{[\s\S]*setSnapshot\(next\);[\s\S]*setInitialLoadError\(null\)/);
  assert.doesNotMatch(receiver, /loadSnapshot\(|setLoading\(true\)/);
});

test("first-run workspace actions stay neutral while retaining hover and keyboard focus", () => {
  const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
  const actions = [...page.matchAll(/className="(file-space-setup-option(?:\s[^"]*)?)"/g)];
  assert.equal(actions.length, 2);
  for (const [, className] of actions) assert.equal(className, "file-space-setup-option");
  assert.doesNotMatch(css, /\.file-space-setup-option\.is-primary/);
  assert.match(css, /\.file-space-setup-option:hover:not\(:disabled\)\s*\{/);
  assert.match(css, /\.file-space-setup-option:focus-visible,[^{]+\{[^}]*outline:/);
});
