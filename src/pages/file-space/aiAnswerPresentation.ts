export interface AiCitationSource {
  citationId: string;
  fileId: string;
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

export const openBackgroundStatusEventName = "lumetrace:open-background-status";

export interface AiThinkingProgress {
  requestId: string;
  phase: string;
  thinking: string;
}

export function shouldAcceptAiThinkingProgress(
  activeRequestId: string | null | undefined,
  progress: AiThinkingProgress,
) {
  return Boolean(activeRequestId)
    && progress.requestId === activeRequestId
    && progress.phase === "thinking";
}

export function isAiNoSourcesError(errorCode: string | null | undefined) {
  return errorCode === "ai_no_sources";
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
  const referencedFiles: T[] = [];
  const seenFileIds = new Set<string>();

  for (const source of sources) {
    if (seenFileIds.has(source.fileId)) continue;
    seenFileIds.add(source.fileId);
    referencedFiles.push(source);
  }

  return referencedFiles;
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
