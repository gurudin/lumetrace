import assert from "node:assert/strict";
import test from "node:test";
import { defaultVersionComparison, diffTextVersions } from "../src/pages/file-space/versionDiff.ts";
import { previewDiffExceedsRenderBudget, previewDiffMaxBytes, readPreviewDiffPair } from "../src/pages/file-space/previewDiffData.ts";

const versions = [4, 3, 2, 1].map((versionNumber) => ({
  id: `v${versionNumber}`, versionNumber, isCurrent: versionNumber === 4, sizeBytes: 20,
}));

test("preview Diff anchors to the selected historical version, not the current version", () => {
  assert.deepEqual(defaultVersionComparison(versions, "v3"), { beforeVersionId: "v2", afterVersionId: "v3" });
  assert.deepEqual(defaultVersionComparison(versions, "v4"), { beforeVersionId: "v3", afterVersionId: "v4" });
  assert.deepEqual(defaultVersionComparison(versions, "v1"), { beforeVersionId: "v1", afterVersionId: "v2" });
  assert.equal(defaultVersionComparison([versions[0]], "v4"), null);
  assert.equal(defaultVersionComparison([], ""), null);
});

test("reads exactly two selected saved snapshots, including the current version", async () => {
  const calls: string[] = [];
  const texts: Record<string, string> = { v2: "version one", v4: "version three" };
  const pair = await readPreviewDiffPair("synthetic.md", versions[2], versions[0], async (fileId, versionId) => {
    calls.push(`${fileId}:${versionId}`);
    return Array.from(new TextEncoder().encode(texts[versionId]));
  });
  assert.deepEqual(calls, ["synthetic.md:v2", "synthetic.md:v4"]);
  assert.ok(pair);
  const result = diffTextVersions(new TextDecoder().decode(pair.before), new TextDecoder().decode(pair.after));
  assert.equal(result.addedLines, 1);
  assert.equal(result.removedLines, 1);
  assert.equal(previewDiffExceedsRenderBudget(result), false);
});

test("arbitrary version changes, swapping and identical snapshots remain exact", async () => {
  const read = async (_file: string, versionId: string) => Array.from(new TextEncoder().encode(versionId));
  const pair = await readPreviewDiffPair("synthetic.txt", versions[0], versions[3], read);
  assert.ok(pair);
  assert.equal(new TextDecoder().decode(pair.before), "v4");
  assert.equal(new TextDecoder().decode(pair.after), "v1");
  assert.equal(diffTextVersions("version one", "version one").identical, true);
  await assert.rejects(readPreviewDiffPair("synthetic.txt", versions[0], versions[0], read), /different/);
});

test("oversized metadata blocks snapshot reads and oversized payloads are rechecked", async () => {
  assert.equal(await readPreviewDiffPair("synthetic.md", { ...versions[0], sizeBytes: previewDiffMaxBytes + 1 }, versions[1], async () => {
    assert.fail("must not read oversized snapshots");
  }), null);
  assert.equal(await readPreviewDiffPair("synthetic.md", versions[0], versions[1], async () => Array(previewDiffMaxBytes + 1)), null);
});

test("snapshot failure is reported without substituting the live file or another version", async () => {
  await assert.rejects(readPreviewDiffPair("synthetic.md", versions[0], versions[1], async () => {
    throw new Error("Snapshot unavailable");
  }), /Snapshot unavailable/);
});

test("high-density diffs are bounded before rendering rows or inline marks", () => {
  const result = diffTextVersions("before", "after");
  assert.equal(previewDiffExceedsRenderBudget({ ...result, rows: Array(2_001).fill(result.rows[0]) }), true);
  const longLine = diffTextVersions("a".repeat(500_001), "b");
  assert.equal(previewDiffExceedsRenderBudget(longLine), true);
  const manyMarks = { ...result, rows: [{ kind: "change" as const, before: null,
    after: { lineNumber: 1, text: "text", segments: Array(16_001).fill({ kind: "added", text: "a" }) } }] };
  assert.equal(previewDiffExceedsRenderBudget(manyMarks), true);
});
