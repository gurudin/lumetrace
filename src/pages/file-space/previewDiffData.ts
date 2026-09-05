import type { VersionDiffResult } from "./versionDiff";

export const previewDiffMaxBytes = 5 * 1024 * 1024;

export interface PreviewDiffVersion {
  id: string;
  versionNumber: number;
  isCurrent: boolean;
  sizeBytes: number;
}

export type PreviewSnapshotReader = (fileId: string, versionId: string) => Promise<number[]>;

export async function readPreviewDiffPair(
  fileId: string,
  before: PreviewDiffVersion,
  after: PreviewDiffVersion,
  read: PreviewSnapshotReader,
) {
  if (before.id === after.id) throw new Error("Choose two different recorded versions.");
  if ([before, after].some((version) => version.sizeBytes > previewDiffMaxBytes)) return null;
  // Even the current version is read from its saved snapshot, never the live file or draft.
  const pair = await Promise.all([read(fileId, before.id), read(fileId, after.id)]);
  if (pair.some((bytes) => bytes.length > previewDiffMaxBytes)) return null;
  return { before: new Uint8Array(pair[0]), after: new Uint8Array(pair[1]) };
}

export function previewDiffExceedsRenderBudget(result: VersionDiffResult) {
  if (result.rows.length > 2_000) return true;
  let segments = 0;
  let characters = 0;
  for (const row of result.rows) {
    if (row.kind === "omitted") continue;
    for (const side of [row.before, row.after]) {
      segments += side?.segments.length ?? 0;
      characters += side?.text.length ?? 0;
    }
    if (segments > 16_000 || characters > 500_000) return true;
  }
  return false;
}
