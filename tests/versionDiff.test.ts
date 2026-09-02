import assert from "node:assert/strict";
import test from "node:test";
import { defaultVersionComparison, diffTextVersions } from "../src/pages/file-space/versionDiff.ts";

test("reports inserted and removed lines with surrounding context", () => {
  const result = diffTextVersions(
    "alpha\nbeta\ngamma\ndelta\nepsilon\n",
    "alpha\nbeta changed\ngamma\nepsilon\nzeta\n",
    1,
  );

  assert.equal(result.identical, false);
  assert.equal(result.removedLines, 2);
  assert.equal(result.addedLines, 2);
  assert.ok(result.rows.some((row) => row.kind === "change" && row.before?.text === "beta" && row.after?.text === "beta changed"));
  assert.ok(result.rows.some((row) => row.kind === "change" && row.before?.text === "delta"));
  assert.ok(result.rows.some((row) => row.kind === "change" && row.after?.text === "zeta"));
});

test("marks changed words inside a paired line", () => {
  const result = diffTextVersions("File version one", "File version two");
  const changed = result.rows.find((row) => row.kind === "change" && row.before && row.after);

  assert.ok(changed && changed.kind === "change");
  assert.deepEqual(changed.before?.segments.map((segment) => [segment.kind, segment.text]), [
    ["equal", "File"],
    ["equal", " "],
    ["equal", "version"],
    ["equal", " "],
    ["removed", "one"],
  ]);
  assert.deepEqual(changed.after?.segments.at(-1), { kind: "added", text: "two" });
});

test("collapses unchanged regions outside the requested context", () => {
  const before = Array.from({ length: 20 }, (_, index) => `line ${index + 1}`).join("\n");
  const after = before.replace("line 10", "line ten");
  const result = diffTextVersions(before, after, 2);

  assert.equal(result.rows.filter((row) => row.kind === "omitted").length, 2);
  assert.ok(result.rows.length < 10);
});

test("returns an empty rendered diff when versions are identical", () => {
  const result = diffTextVersions("same\ncontent\n", "same\ncontent\n");
  assert.equal(result.identical, true);
  assert.equal(result.addedLines, 0);
  assert.equal(result.removedLines, 0);
  assert.deepEqual(result.rows, []);
});

test("defaults to the version immediately before the current version", () => {
  assert.deepEqual(defaultVersionComparison([
    { id: "v3", versionNumber: 3, isCurrent: true },
    { id: "v2", versionNumber: 2, isCurrent: false },
    { id: "v1", versionNumber: 1, isCurrent: false },
  ], "v3"), { beforeVersionId: "v2", afterVersionId: "v3" });

  assert.deepEqual(defaultVersionComparison([
    { id: "v3", versionNumber: 3, isCurrent: false },
    { id: "v2", versionNumber: 2, isCurrent: true },
    { id: "v1", versionNumber: 1, isCurrent: false },
  ], "v2"), { beforeVersionId: "v1", afterVersionId: "v2" });

  assert.deepEqual(defaultVersionComparison([
    { id: "v2", versionNumber: 2, isCurrent: false },
    { id: "v1", versionNumber: 1, isCurrent: true },
  ], "v1"), { beforeVersionId: "v1", afterVersionId: "v2" });
});
