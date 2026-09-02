import assert from "node:assert/strict";
import test from "node:test";
import { resolveFileDoubleClickRoute } from "../src/pages/file-space/fileOpenRouting.ts";

test("routes built-in preview formats away from the external-file bridge", () => {
  for (const fileName of ["report.pdf", "notes.md", "README.MARKDOWN", "draft.txt", "  mixed.PdF  "]) {
    assert.equal(resolveFileDoubleClickRoute(fileName), "internal-preview");
  }
  for (const fileName of ["photo.png", "photo.jpg", "photo.jpeg", "photo.gif", "photo.webp"]) {
    assert.equal(resolveFileDoubleClickRoute(fileName, true), "internal-preview");
    assert.equal(resolveFileDoubleClickRoute(fileName, false), "reveal");
  }
});

test("keeps office and web documents on the external application path", () => {
  for (const fileName of [
    "brief.doc", "brief.docx", "sheet.xls", "sheet.XLSX", "deck.ppt", "deck.pptx",
    "page.htm", "page.html", "data.csv",
  ]) {
    assert.equal(resolveFileDoubleClickRoute(fileName), "external-open");
  }
});

test("reveals unsupported files when no internal preview is available", () => {
  assert.equal(resolveFileDoubleClickRoute("archive.zip"), "reveal");
  assert.equal(resolveFileDoubleClickRoute("image-without-preview.heic"), "reveal");
  assert.equal(resolveFileDoubleClickRoute("report.pdf.zip"), "reveal");
  assert.equal(resolveFileDoubleClickRoute("README"), "reveal");
});
