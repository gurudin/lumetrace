import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileRenameSelectionEnd } from "../src/pages/file-space/fileRenameSelection.ts";

test("file rename leaves the final extension outside the initial selection", () => {
  for (const [name, selected, suffix] of [
    ["1e825fa3734b4255bbcf5736d01248d9.jpeg", "1e825fa3734b4255bbcf5736d01248d9", ".jpeg"],
    ["photo.JPG", "photo", ".JPG"],
    ["release.v2.final.pdf", "release.v2.final", ".pdf"],
    ["我的照片📷.png", "我的照片📷", ".png"],
    [".env.local", ".env", ".local"],
  ]) {
    const end = fileRenameSelectionEnd(name);
    assert.equal(name.slice(0, end), selected);
    assert.equal(name.slice(end), suffix);
    assert.equal("New name" + name.slice(end), "New name" + suffix);
  }
});

test("names without extensions and plain dotfiles stay fully selected", () => {
  for (const name of ["README", ".env", ".gitignore", "draft.", "", "报告📷"]) {
    assert.equal(fileRenameSelectionEnd(name), name.length);
  }
});

test("the shared file dialog applies basename selection without locking the input", () => {
  const page = readFileSync(new URL("../src/pages/file-space/FileSpacePage.tsx", import.meta.url), "utf8");
  const start = page.indexOf("if (!renameFileId || !renameFileDialogPresence.mounted) return;");
  const end = page.indexOf("[renameFileDialogPresence.mounted, renameFileId]", start);
  const effect = page.slice(start, end);
  assert.match(effect, /input\.focus\(\)/);
  assert.match(effect, /input\.setSelectionRange\(0, fileRenameSelectionEnd\(input\.value\)\)/);
  assert.match(effect, /cancelAnimationFrame\(frame\)/);
  assert.doesNotMatch(effect, /\.select\(\)/);
  assert.match(page, /<input ref=\{renameFileNameRef\} value=\{renameFileName\} onChange=\{\(event\) => setRenameFileName\(event\.target\.value\)\} \/>/);
  assert.match(page, /renameFolderNameRef\.current\?\.select\(\)/);
});
