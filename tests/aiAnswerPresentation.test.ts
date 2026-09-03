import assert from "node:assert/strict";
import test from "node:test";
import {
  aiAnswerDurationSeconds,
  aiPendingStatusKey,
  aiPendingElapsedSeconds,
  isAiNoSourcesError,
  openBackgroundStatusEventName,
  referencedAiFiles,
  shouldAcceptAiProgress,
  shouldSelectAiSourceFromClickDetail,
  visibleAiAnswer,
} from "../src/pages/file-space/aiAnswerPresentation.ts";

const sources = [
  { citationId: "S1", fileId: "file-a", fileName: "A.md", evidenceRole: "context" as const, citationCount: 1 },
  { citationId: "S2", fileId: "file-a", fileName: "A.md", evidenceRole: "primary" as const, citationCount: 2 },
  { citationId: "S3", fileId: "file-b", fileName: "B.docx", evidenceRole: "context" as const, citationCount: 1 },
];

test("removes citation markers from the visible AI answer", () => {
  assert.equal(
    visibleAiAnswer("第一项 [S1][S2]，第二项 [S3]。"),
    "第一项，第二项。",
  );
});

test("aggregates citations by file and sorts primary evidence first", () => {
  assert.deepEqual(referencedAiFiles(sources).map((source) => ({
    fileName: source.fileName,
    evidenceRole: source.evidenceRole,
    citationCount: source.citationCount,
  })), [
    { fileName: "A.md", evidenceRole: "primary", citationCount: 3 },
    { fileName: "B.docx", evidenceRole: "context", citationCount: 1 },
  ]);
});

test("rounds answer processing time up to whole seconds", () => {
  assert.equal(aiAnswerDurationSeconds(1_001, 0, 0), 2);
  assert.equal(aiAnswerDurationSeconds(0, 0, 0), 1);
});

test("falls back to turn timestamps for legacy answers", () => {
  assert.equal(aiAnswerDurationSeconds(null, 1_000, 3_400), 3);
});

test("keeps pending answer time anchored when the panel is reopened", () => {
  const startedAt = 1_000;
  assert.equal(aiPendingElapsedSeconds(startedAt, 1_900), 0);
  assert.equal(aiPendingElapsedSeconds(startedAt, 13_400), 12);
  assert.equal(aiPendingElapsedSeconds(null, 13_400), 0);
});

test("treats no relevant sources as a non-retryable empty answer state", () => {
  assert.equal(isAiNoSourcesError("ai_no_sources"), true);
  assert.equal(isAiNoSourcesError("ai_search_failed"), false);
  assert.equal(isAiNoSourcesError(null), false);
});

test("uses one event contract to open background task status", () => {
  assert.equal(openBackgroundStatusEventName, "lumetrace:open-background-status");
});

test("AI source clicks select once while leaving double-click to open", () => {
  assert.equal(shouldSelectAiSourceFromClickDetail(0), true);
  assert.equal(shouldSelectAiSourceFromClickDetail(1), true);
  assert.equal(shouldSelectAiSourceFromClickDetail(2), false);
});

test("AI progress events update only their active request", () => {
  const progress = {
    requestId: "request-current",
    phase: "thinking" as const,
    thinking: "checking sources",
  };
  assert.equal(shouldAcceptAiProgress("request-current", progress), true);
  assert.equal(shouldAcceptAiProgress("request-old", progress), false);
  assert.equal(shouldAcceptAiProgress(null, progress), false);
  assert.equal(
    shouldAcceptAiProgress("request-current", { ...progress, phase: "answer" as never }),
    false,
  );
});

test("AI progress phases map to distinct waiting copy", () => {
  assert.equal(aiPendingStatusKey("retrieving"), "asking");
  assert.equal(aiPendingStatusKey("generating"), "generating");
  assert.equal(aiPendingStatusKey("thinking"), "thinking");
});
