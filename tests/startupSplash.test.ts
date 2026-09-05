import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startupSplashId } from "../src/shared/ui/startupSplash.ts";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const tauriConfig = JSON.parse(
  readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
) as { app: { windows: Array<{ visible?: boolean }> } };
const tauriEntry = readFileSync(
  new URL("../src-tauri/src/lib.rs", import.meta.url),
  "utf8",
);

test("renders the startup surface before the React root", () => {
  const splashPosition = indexHtml.indexOf(`id="${startupSplashId}"`);
  const rootPosition = indexHtml.indexOf('id="root"');

  assert.ok(splashPosition >= 0);
  assert.ok(rootPosition > splashPosition);
  assert.match(indexHtml, /lume-trace-startup-spinner/);
  assert.match(indexHtml, /src-tauri\/icons\/icon\.png/);
});

test("resolves the saved appearance before the startup surface is painted", () => {
  assert.match(indexHtml, /lumetrace\.theme\.preference/);
  assert.match(indexHtml, /prefers-color-scheme: dark/);
  assert.match(indexHtml, /lumetrace\.appearance/);
  assert.match(indexHtml, /prefers-reduced-motion: reduce/);
});

test("keeps the native window hidden until the themed startup surface is ready", () => {
  assert.equal(tauriConfig.app.windows[0]?.visible, false);
  assert.match(tauriEntry, /AtomicBool/);
  assert.match(tauriEntry, /PageLoadEvent::Finished/);
  assert.match(tauriEntry, /ready_for_page\.swap\(true, Ordering::AcqRel\)/);
  assert.match(tauriEntry, /ready_for_instance\.load\(Ordering::Acquire\)/);
  assert.match(tauriEntry, /ready_for_reopen\.load\(Ordering::Acquire\)/);
});
