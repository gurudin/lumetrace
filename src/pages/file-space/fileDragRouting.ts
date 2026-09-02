export type FileDragPhase = "pending" | "reordering";
export type FileDragHandoffTrigger = "command" | "blur";
export type FileDragHandoffDecision = "promote" | "cancel" | "ignore";

export function canActivateFileReorder({
  hasSearch,
  hasFilters,
  usesAutomaticOrder,
  visibleFileCount,
}: {
  hasSearch: boolean;
  hasFilters: boolean;
  usesAutomaticOrder: boolean;
  visibleFileCount: number;
}) {
  return !hasSearch
    && !hasFilters
    && usesAutomaticOrder
    && visibleFileCount > 1;
}

export function hasMeaningfulFileReorderMovement({
  startX,
  startY,
  currentX,
  currentY,
  minimumDistance,
}: {
  startX: number;
  startY: number;
  currentX: number;
  currentY: number;
  minimumDistance: number;
}) {
  return Math.hypot(currentX - startX, currentY - startY) >= minimumDistance;
}

export function decideFileDragHandoff({
  phase,
  fileCount,
  desktop,
  trigger,
}: {
  phase: FileDragPhase;
  fileCount: number;
  desktop: boolean;
  trigger: FileDragHandoffTrigger;
}): FileDragHandoffDecision {
  const canPromote = phase === "reordering" && fileCount === 1 && desktop;
  if (canPromote) return "promote";
  return trigger === "blur" ? "cancel" : "ignore";
}
