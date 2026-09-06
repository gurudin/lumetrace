import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { applyVersionAnnotation, versionNoteLimit, versionNoteLength } from "../src/pages/file-space/versionAnnotationState.ts";

const timeline = { workspaceId: "space-a", fileId: "file", currentVersionId: "v3", versions: [
  { id: "v3", versionNumber: 3, note: "", isMilestone: false },
  { id: "v2", versionNumber: 2, note: "", isMilestone: false },
  { id: "v1", versionNumber: 1, note: "initial", isMilestone: true },
] };
const update = { workspaceId: "space-a", fileId: "file", versionId: "v2", note: "Approved release", isMilestone: true };

test("version annotations update exactly one version without changing order or the current version", () => {
  const next = applyVersionAnnotation(timeline, update)!;
  assert.equal(next.currentVersionId, "v3");
  assert.deepEqual(next.versions.map((version) => version.id), ["v3", "v2", "v1"]);
  assert.equal(next.versions[0], timeline.versions[0]);
  assert.equal(next.versions[2], timeline.versions[2]);
  assert.equal(next.versions[1].note, "Approved release");
  assert.equal(next.versions[1].isMilestone, true);
  assert.equal(timeline.versions[1].note, "");
});

test("late annotation responses cannot cross workspace, file or version boundaries", () => {
  for (const patch of [{ workspaceId: "space-b" }, { fileId: "other" }, { versionId: "missing" }]) {
    assert.equal(applyVersionAnnotation(timeline, { ...update, ...patch }), timeline);
  }
  assert.equal(applyVersionAnnotation(null, update), null);
  const oldBackend = { ...timeline, workspaceId: undefined };
  assert.equal(applyVersionAnnotation(oldBackend, update), oldBackend);
});

test("clearing notes and unmarking milestones preserve all other version metadata", () => {
  const changed = applyVersionAnnotation(timeline, update)!;
  const cleared = applyVersionAnnotation(changed, { ...update, note: "", isMilestone: false })!;
  assert.equal(cleared.versions[1].note, "");
  assert.equal(cleared.versions[1].isMilestone, false);
  assert.equal(cleared.versions[1].versionNumber, 2);
});

test("each timeline entry shares the annotation controls and keyboard-safe editor", () => {
  for (const file of ["FileSpacePage", "MarkdownPreviewOverlay", "TextPreviewOverlay", "TaskVersionTimelineRail"]) {
    const source = readFileSync(new URL(`../src/pages/file-space/${file}.tsx`, import.meta.url), "utf8");
    assert.match(source, /<VersionAnnotation workspaceId=/);
  }
  const editor = readFileSync(new URL("../src/pages/file-space/VersionAnnotation.tsx", import.meta.url), "utf8");
  assert.match(editor, /showModal\(\)/);
  assert.match(editor, /aria-pressed=/);
  assert.match(editor, /event\.stopPropagation\(\)/);
  assert.match(editor, /if \(noteTooLong\) return/);
  assert.match(editor, /disabled=\{busy \|\| noteTooLong\}/);
  assert.match(editor, /data-native-context-menu="true"/);
  assert.match(editor, /request: \{ workspaceId, fileId, versionId: version.id, \.\.\.patch \}/);
});

test("notes allow 50 Unicode scalars and preserve over-limit drafts for correction", () => {
  assert.equal(versionNoteLimit, 50);
  for (const character of ["a", "版", "🌟"]) {
    assert.equal(versionNoteLength(character.repeat(50)), 50);
    assert.equal(versionNoteLength(character.repeat(51)), 51);
  }
  const editor = readFileSync(new URL("../src/pages/file-space/VersionAnnotation.tsx", import.meta.url), "utf8");
  assert.match(editor, /setDraft\(version.note \?\? ""\)/);
  assert.doesNotMatch(editor, /draft\.slice|maxLength=|1000/);
});
