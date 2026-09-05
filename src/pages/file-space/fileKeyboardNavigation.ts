import type { JustifiedFilePlacement } from "./fileJustifiedLayout";

export type FileKeyboardDirection = "left" | "right" | "up" | "down";
export type FileKeyboardShortcutAction = "preview" | "trash";

interface FileKeyboardShortcutInput {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat: boolean;
  isComposing: boolean;
}

export function fileKeyboardShortcutAction({
  key,
  metaKey,
  ctrlKey,
  altKey,
  shiftKey,
  repeat,
  isComposing,
}: FileKeyboardShortcutInput): FileKeyboardShortcutAction | null {
  if (repeat || isComposing || ctrlKey || altKey || shiftKey) return null;
  if (key === " " && !metaKey) return "preview";
  if (metaKey && (key === "Backspace" || key === "Delete")) return "trash";
  return null;
}

interface FileKeyboardNavigationOptions {
  orderedIds: string[];
  placements: Record<string, JustifiedFilePlacement>;
  currentId: string | null;
  direction: FileKeyboardDirection;
  layoutMode: "adaptive" | "list";
}

interface VisualFileItem {
  id: string;
  x: number;
  y: number;
  centerX: number;
}

export function nextFileIdForKeyboard({
  orderedIds,
  placements,
  currentId,
  direction,
  layoutMode,
}: FileKeyboardNavigationOptions) {
  const items = orderedIds.flatMap((id): VisualFileItem[] => {
    const placement = placements[id];
    return placement ? [{
      id,
      x: placement.x,
      y: placement.y,
      centerX: placement.x + placement.width / 2,
    }] : [];
  });
  if (items.length === 0) return null;
  if (!currentId || !items.some((item) => item.id === currentId)) return items[0].id;
  if (layoutMode === "list") {
    const currentIndex = items.findIndex((item) => item.id === currentId);
    if (direction === "up") return items[Math.max(0, currentIndex - 1)].id;
    if (direction === "down") return items[Math.min(items.length - 1, currentIndex + 1)].id;
    return null;
  }

  const rows: VisualFileItem[][] = [];
  [...items]
    .sort((left, right) => left.y - right.y || left.x - right.x)
    .forEach((item) => {
      const row = rows.at(-1);
      if (!row || Math.abs(row[0].y - item.y) > 1) rows.push([item]);
      else row.push(item);
    });
  const rowIndex = rows.findIndex((row) => row.some((item) => item.id === currentId));
  const row = rows[rowIndex];
  const itemIndex = row.findIndex((item) => item.id === currentId);
  const current = row[itemIndex];

  if (direction === "left") {
    if (itemIndex > 0) return row[itemIndex - 1].id;
    return rowIndex > 0 ? rows[rowIndex - 1].at(-1)!.id : currentId;
  }
  if (direction === "right") {
    if (itemIndex < row.length - 1) return row[itemIndex + 1].id;
    return rowIndex < rows.length - 1 ? rows[rowIndex + 1][0].id : currentId;
  }

  const adjacentRowIndex = direction === "up" ? rowIndex - 1 : rowIndex + 1;
  const adjacentRow = rows[adjacentRowIndex];
  if (!adjacentRow) return currentId;
  return adjacentRow.reduce((nearest, candidate) => (
    Math.abs(candidate.centerX - current.centerX) < Math.abs(nearest.centerX - current.centerX)
      ? candidate
      : nearest
  )).id;
}
