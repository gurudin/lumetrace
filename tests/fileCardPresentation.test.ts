import assert from "node:assert/strict";
import test from "node:test";
import {
  fileCardMetadataText,
  fileDocumentArtworkFormat,
  shouldShowFileVersionBadge,
} from "../src/pages/file-space/fileCardPresentation.ts";

test("uses the compact MD label for both Markdown extensions", () => {
  assert.equal(fileDocumentArtworkFormat("md"), "md");
  assert.equal(fileDocumentArtworkFormat("markdown"), "md");
});

test("shows a version badge only when a file has multiple versions", () => {
  assert.equal(shouldShowFileVersionBadge(0), false);
  assert.equal(shouldShowFileVersionBadge(1), false);
  assert.equal(shouldShowFileVersionBadge(2), true);
  assert.equal(shouldShowFileVersionBadge(3), true);
});

test("shows only file size for non-images", () => {
  assert.equal(
    fileCardMetadataText("5 KB", false, 100, { fileUpdatedAt: 100, width: 144, height: 144 }),
    "5 KB",
  );
});

test("adds current image dimensions after file size", () => {
  assert.equal(
    fileCardMetadataText("5 KB", true, 100, { fileUpdatedAt: 100, width: 144, height: 144 }),
    "5 KB · 144 × 144",
  );
  assert.equal(
    fileCardMetadataText("5 KB", true, 101, { fileUpdatedAt: 100, width: 144, height: 144 }),
    "5 KB",
  );
});
