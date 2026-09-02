export type FileSelectionMode = "replace" | "union" | "toggle";

export interface FileSelectionResult {
  ids: Set<string>;
  anchorId: string | null;
}

export interface SelectionRectangle {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface ScrollSelectionBounds {
  left: number;
  top: number;
  scrollLeft: number;
  scrollTop: number;
  scrollWidth: number;
  scrollHeight: number;
}

export function setsEqual<T>(left: ReadonlySet<T>, right: ReadonlySet<T>) {
  if (left.size !== right.size) return false;
  for (const value of left) {
    if (!right.has(value)) return false;
  }
  return true;
}

export function orderedSelection(
  orderedIds: readonly string[],
  selectedIds: ReadonlySet<string>,
) {
  return orderedIds.filter((id) => selectedIds.has(id));
}

export function pruneSelection(
  selectedIds: ReadonlySet<string>,
  visibleIds: ReadonlySet<string>,
) {
  return new Set([...selectedIds].filter((id) => visibleIds.has(id)));
}

export function selectionVisibilityWithPendingReveal(
  visibleIds: Iterable<string>,
  pendingRevealId: string | null | undefined,
) {
  const selectableIds = new Set(visibleIds);
  if (pendingRevealId) selectableIds.add(pendingRevealId);
  return selectableIds;
}

export function resolveFileClickSelection({
  currentIds,
  orderedIds,
  targetId,
  anchorId,
  toggle,
  range,
}: {
  currentIds: ReadonlySet<string>;
  orderedIds: readonly string[];
  targetId: string;
  anchorId: string | null;
  toggle: boolean;
  range: boolean;
}): FileSelectionResult {
  if (range) {
    const anchorIndex = anchorId ? orderedIds.indexOf(anchorId) : -1;
    const targetIndex = orderedIds.indexOf(targetId);
    if (anchorIndex >= 0 && targetIndex >= 0) {
      const start = Math.min(anchorIndex, targetIndex);
      const end = Math.max(anchorIndex, targetIndex);
      const ids = new Set(orderedIds.slice(start, end + 1));
      if (toggle) currentIds.forEach((id) => ids.add(id));
      return { ids, anchorId };
    }
  }

  if (toggle) {
    const ids = new Set(currentIds);
    if (ids.has(targetId)) ids.delete(targetId);
    else ids.add(targetId);
    return { ids, anchorId: targetId };
  }

  return { ids: new Set([targetId]), anchorId: targetId };
}

export function normalizeSelectionRectangle(
  startX: number,
  startY: number,
  endX: number,
  endY: number,
): SelectionRectangle {
  return {
    left: Math.min(startX, endX),
    top: Math.min(startY, endY),
    right: Math.max(startX, endX),
    bottom: Math.max(startY, endY),
  };
}

export function pointInScrollContent(
  clientX: number,
  clientY: number,
  bounds: ScrollSelectionBounds,
) {
  return {
    x: Math.max(0, Math.min(
      clientX - bounds.left + bounds.scrollLeft,
      bounds.scrollWidth,
    )),
    y: Math.max(0, Math.min(
      clientY - bounds.top + bounds.scrollTop,
      bounds.scrollHeight,
    )),
  };
}

export function rectanglesIntersect(
  left: SelectionRectangle,
  right: SelectionRectangle,
) {
  return left.left < right.right
    && left.right > right.left
    && left.top < right.bottom
    && left.bottom > right.top;
}

export function combineMarqueeSelection(
  baselineIds: ReadonlySet<string>,
  intersectingIds: ReadonlySet<string>,
  mode: FileSelectionMode,
) {
  if (mode === "replace") return new Set(intersectingIds);
  const ids = new Set(baselineIds);
  intersectingIds.forEach((id) => {
    if (mode === "toggle" && ids.has(id)) ids.delete(id);
    else ids.add(id);
  });
  return ids;
}
