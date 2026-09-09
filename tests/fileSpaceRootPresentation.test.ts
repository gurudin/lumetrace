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
