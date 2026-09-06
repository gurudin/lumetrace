export interface AnnotatedVersion { id: string; note?: string; isMilestone?: boolean; }
export interface AnnotatedTimeline { workspaceId?: string; fileId: string; versions: AnnotatedVersion[]; }
export interface VersionAnnotationUpdate {
  workspaceId: string;
  fileId: string;
  versionId: string;
  note: string;
  isMilestone: boolean;
}
export const versionAnnotationUpdatedEvent = "file-space-version-annotation-updated";
export const versionNoteLimit = 50;
// Match Rust's Unicode scalar count; a surrogate pair is not two characters.
export const versionNoteLength = (note: string) => Array.from(note).length;

export function applyVersionAnnotation<T extends AnnotatedTimeline>(timeline: T | null, update: VersionAnnotationUpdate): T | null {
  if (!timeline || timeline.workspaceId !== update.workspaceId || timeline.fileId !== update.fileId
    || !timeline.versions.some((version) => version.id === update.versionId)) return timeline;
  return { ...timeline, versions: timeline.versions.map((version) => version.id === update.versionId
    ? { ...version, note: update.note, isMilestone: update.isMilestone } : version) };
}
