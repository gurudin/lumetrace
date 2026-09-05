export interface AiCitationSource {
  citationId: string;
  fileId: string;
  evidenceRole: "primary" | "context";
  citationCount: number;
}

export interface FileSpaceAiSourceReference extends AiCitationSource {
  fileName: string;
  relativePath: string;
  versionId: string | null;
  versionNumber: number | null;
  excerpt: string;
  lexicalMatch: boolean;
  semanticSimilarity: number | null;
}

export type AiProgressPhase = "planning" | "locating" | "versions" | "comparing" | "retrieving" | "generating" | "thinking";

export interface AiProgress {
  requestId: string;
  phase: AiProgressPhase;
  thinking: string;
}

export function shouldAcceptAiProgress(
  activeRequestId: string | null | undefined,
  progress: AiProgress,
) {
  return Boolean(activeRequestId)
    && progress.requestId === activeRequestId
    && ["planning", "locating", "versions", "comparing", "retrieving", "generating", "thinking"].includes(progress.phase);
}

export function aiPendingStatusKey(phase: AiProgressPhase) {
  if (phase === "planning" || phase === "locating" || phase === "versions" || phase === "comparing") return phase;
  if (phase === "generating") return "generating";
  if (phase === "thinking") return "thinking";
  return "asking";
}

export function isAiNoSourcesError(errorCode: string | null | undefined) {
  return errorCode === "ai_no_sources";
}

export function isAiCancelledError(errorCode: string | null | undefined) {
  return errorCode === "ai_cancelled";
}

export function shouldSelectAiSourceFromClickDetail(detail: number) {
  return detail < 2;
}

export function visibleAiAnswer(answer: string) {
  return answer
    .replace(/[ \t]*\[S\d+\]/g, "")
    .replace(/[ \t]+([,.;:!?，。；：！？、])/g, "$1")
    .trim();
}

export function referencedAiFiles<T extends AiCitationSource>(sources: readonly T[]) {
  const referencedFiles = new Map<string, T>();

  for (const source of sources) {
    const existing = referencedFiles.get(source.fileId);
    if (!existing) {
      referencedFiles.set(source.fileId, { ...source });
      continue;
    }
    referencedFiles.set(source.fileId, {
      ...existing,
      evidenceRole: existing.evidenceRole === "primary" || source.evidenceRole === "primary"
        ? "primary"
        : "context",
      citationCount: existing.citationCount + source.citationCount,
    });
  }

  return [...referencedFiles.values()].sort((left, right) => {
    const roleDifference = Number(left.evidenceRole === "context")
      - Number(right.evidenceRole === "context");
    if (roleDifference !== 0) return roleDifference;
    return right.citationCount - left.citationCount;
  });
}

export function aiAnswerDurationSeconds(
  durationMs: number | null | undefined,
  createdAt: number,
  updatedAt: number,
) {
  const elapsedMs = typeof durationMs === "number" && Number.isFinite(durationMs)
    ? durationMs
    : updatedAt - createdAt;
  return Math.max(1, Math.ceil(Math.max(0, elapsedMs) / 1_000));
}

export function aiPendingElapsedSeconds(
  startedAt: number | null | undefined,
  now = Date.now(),
) {
  if (typeof startedAt !== "number" || !Number.isFinite(startedAt)) return 0;
  return Math.max(0, Math.floor((now - startedAt) / 1_000));
}
