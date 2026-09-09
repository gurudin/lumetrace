import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderApplicationHtml } from "../build/applicationHtml.ts";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

test("both editions retain the same startup theme, motion and root contract", () => {
  for (const assetPrefix of ["", "/vendor/lumetrace"]) {
    const html = renderApplicationHtml(read("index.html"), { displayName: "LumeTrace", assetPrefix });
    assert.match(html, /lumetrace\.theme\.preference/);
    assert.match(html, /prefers-reduced-motion/);
    assert.ok(html.indexOf('id="lume-trace-startup"') < html.indexOf('id="root"'));
    assert.ok(html.includes(`src="${assetPrefix}/src-tauri/icons/icon.png"`));
    assert.match(html, /src="\/src\/main\.tsx"/);
    assert.match(html, /<title>LumeTrace<\/title>/);
  }
});

test("edition metadata is escaped without changing shared startup logic", () => {
  const html = renderApplicationHtml(read("index.html"), {
    displayName: 'LumeTrace Pro <"test">', assetPrefix: "/vendor/lumetrace",
  });
  assert.match(html, /LumeTrace Pro &lt;&quot;test&quot;&gt;/);
  assert.equal((html.match(/<script/g) ?? []).length, 2);
});

test("community and shared library keep separate context ownership", () => {
  const library = read("src-tauri/src/lib.rs");
  assert.doesNotMatch(library, /generate_context!/);
  assert.match(library, /build\(context\)/);
  assert.match(library, /run_with_plugins/);
  assert.match(library, /invoke_handler\(tauri::generate_handler!/);
  assert.match(read("src-tauri/src/main.rs"), /run\(tauri::generate_context!\(\)\)/);
  const config = JSON.parse(read("src-tauri/tauri.conf.json"));
  assert.equal(config.identifier, "com.lumetrace.desktop");
});

test("the community entry uses the shared UI and shared native menu handling", () => {
  assert.match(read("src/main.tsx"), /mountLumeTrace/);
  assert.doesNotMatch(read("src/main.tsx"), /createRoot/);
  assert.match(read("src/bootstrap.tsx"), /shouldPreserveNativeContextMenuForTarget/);
  assert.match(read("src/bootstrap.tsx"), /removeEventListener/);
  assert.match(read("src/styles.css"), /@source "\.\/"/);
});

test("workspace popover targets the selected radio before falling back to the first choice", () => {
  const source = read("src/pages/file-space/FileSpaceWorkspaceSwitcher.tsx");
  assert.match(source, /\[aria-checked="true"\]:not\(:disabled\)/);
  assert.match(source, /selected \?\? menu\.current\?\.querySelector/);
  assert.match(source, /selected\?\.scrollIntoView\(\{ block: "nearest" \}\)/);
});

test("external workspace identity has an optional icon without changing local status layout", () => {
  assert.match(read("src/shared/extensions/ApplicationExtension.tsx"), /icon\?: ReactNode/);
  const status = read("src/pages/file-space/FileSpaceWorkspaceMenu.tsx");
  assert.match(status, /external\.icon \? " has-icon" : ""/);
  assert.match(status, /file-space-health-dot/);
  assert.match(read("src/styles.css"), /\.file-space-workspace-status\.has-icon\s*\{\s*grid-template-columns: 28px minmax\(0, 1fr\)/);
});
