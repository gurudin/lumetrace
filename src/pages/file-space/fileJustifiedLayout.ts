export interface JustifiedFileItem {
  id: string;
  aspectRatio: number;
}

export interface JustifiedFilePlacement {
  x: number;
  y: number;
  width: number;
  previewHeight: number;
}

export interface JustifiedFileLayout {
  previewHeight: number;
  detailsHeight: number;
  cardHeight: number;
  height: number;
  rowCount: number;
  placements: Record<string, JustifiedFilePlacement>;
}

export interface FileReorderCandidateRect {
  id: string;
  left: number;
  right: number;
  top: number;
  bottom: number;
}

interface FileReorderRow {
  top: number;
  bottom: number;
  items: Array<FileReorderCandidateRect & { centerX: number }>;
}

interface CalculateJustifiedFileLayoutOptions {
  containerWidth: number;
  targetPreviewHeight: number;
  horizontalGap: number;
  verticalGap: number;
  previewDetailsGap: number;
  detailsHeight: number;
  minAspectRatio?: number;
  maxAspectRatio?: number;
  items: JustifiedFileItem[];
}

interface CalculateFileListLayoutOptions {
  containerWidth: number;
  rowHeight: number;
  verticalGap: number;
  previewSize: number;
  items: JustifiedFileItem[];
}

interface PreparedItem {
  id: string;
  width: number;
}

function finiteNonNegative(value: number) {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(maximum, Math.max(minimum, value));
}

export function calculateJustifiedFileLayout({
  containerWidth,
  targetPreviewHeight,
  horizontalGap,
  verticalGap,
  previewDetailsGap,
  detailsHeight,
  minAspectRatio = 0.58,
  maxAspectRatio = 2.5,
  items,
}: CalculateJustifiedFileLayoutOptions): JustifiedFileLayout {
  const width = finiteNonNegative(containerWidth);
  const previewHeight = finiteNonNegative(targetPreviewHeight);
  const columnGap = finiteNonNegative(horizontalGap);
  const rowGap = finiteNonNegative(verticalGap);
  const copyGap = finiteNonNegative(previewDetailsGap);
  const copyHeight = finiteNonNegative(detailsHeight);
  const cardHeight = previewHeight + copyGap + copyHeight;
  const preparedItems = items.map((item): PreparedItem => {
    const safeRatio = Number.isFinite(item.aspectRatio) && item.aspectRatio > 0
      ? clamp(item.aspectRatio, minAspectRatio, maxAspectRatio)
      : 1;
    return {
      id: item.id,
      width: Math.min(width, previewHeight * safeRatio),
    };
  });
  const rows: PreparedItem[][] = [];
  let currentRow: PreparedItem[] = [];
  let currentRowWidth = 0;

  preparedItems.forEach((item) => {
    const nextWidth = currentRowWidth + (currentRow.length > 0 ? columnGap : 0) + item.width;
    if (currentRow.length > 0 && nextWidth > width) {
      rows.push(currentRow);
      currentRow = [item];
      currentRowWidth = item.width;
      return;
    }
    currentRow.push(item);
    currentRowWidth = nextWidth;
  });
  if (currentRow.length > 0) rows.push(currentRow);

  const placements: Record<string, JustifiedFilePlacement> = {};
  rows.forEach((row, rowIndex) => {
    const availableWidth = Math.max(0, width - columnGap * Math.max(0, row.length - 1));
    const baseWidth = row.reduce((total, item) => total + item.width, 0);
    const shouldFillRow = rowIndex < rows.length - 1;
    const extraWidth = shouldFillRow && row.length > 0
      ? Math.max(0, availableWidth - baseWidth) / row.length
      : 0;
    let x = 0;
    let assignedWidth = 0;

    row.forEach((item, itemIndex) => {
      const isLastItem = itemIndex === row.length - 1;
      const itemWidth = shouldFillRow && isLastItem
        ? Math.max(0, availableWidth - assignedWidth)
        : item.width + extraWidth;
      placements[item.id] = {
        x,
        y: rowIndex * (cardHeight + rowGap),
        width: itemWidth,
        previewHeight,
      };
      assignedWidth += itemWidth;
      x += itemWidth + columnGap;
    });
  });

  return {
    previewHeight,
    detailsHeight: copyHeight,
    cardHeight,
    height: rows.length > 0
      ? rows.length * cardHeight + Math.max(0, rows.length - 1) * rowGap
      : 0,
    rowCount: rows.length,
    placements,
  };
}

export function calculateFileListLayout({
  containerWidth,
  rowHeight,
  verticalGap,
  previewSize,
  items,
}: CalculateFileListLayoutOptions): JustifiedFileLayout {
  const width = finiteNonNegative(containerWidth);
  const cardHeight = finiteNonNegative(rowHeight);
  const rowGap = finiteNonNegative(verticalGap);
  const previewHeight = Math.min(cardHeight, finiteNonNegative(previewSize));
  const placements: Record<string, JustifiedFilePlacement> = {};

  items.forEach((item, index) => {
    placements[item.id] = {
      x: 0,
      y: index * (cardHeight + rowGap),
      width,
      previewHeight,
    };
  });

  return {
    previewHeight,
    detailsHeight: Math.max(0, cardHeight - previewHeight),
    cardHeight,
    height: items.length > 0
      ? items.length * cardHeight + Math.max(0, items.length - 1) * rowGap
      : 0,
    rowCount: items.length,
    placements,
  };
}

export function justifiedFileLayoutsEqual(
  left: JustifiedFileLayout | null,
  right: JustifiedFileLayout,
) {
  if (!left) return false;
  if (
    left.previewHeight !== right.previewHeight
    || left.detailsHeight !== right.detailsHeight
    || left.cardHeight !== right.cardHeight
    || left.height !== right.height
    || left.rowCount !== right.rowCount
  ) return false;

  const leftIds = Object.keys(left.placements);
  const rightIds = Object.keys(right.placements);
  if (leftIds.length !== rightIds.length) return false;
  return rightIds.every((id) => (
    left.placements[id]?.x === right.placements[id].x
    && left.placements[id]?.y === right.placements[id].y
    && left.placements[id]?.width === right.placements[id].width
    && left.placements[id]?.previewHeight === right.placements[id].previewHeight
  ));
}

export function visibleJustifiedFileIds(
  orderedIds: string[],
  layout: JustifiedFileLayout,
  viewportTop: number,
  viewportBottom: number,
) {
  const top = finiteNonNegative(viewportTop);
  const bottom = Math.max(top, finiteNonNegative(viewportBottom));
  let low = 0;
  let high = orderedIds.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    const placement = layout.placements[orderedIds[middle]];
    if (placement && placement.y + layout.cardHeight < top) low = middle + 1;
    else high = middle;
  }

  const visibleIds = [];
  for (let index = low; index < orderedIds.length; index += 1) {
    const id = orderedIds[index];
    const placement = layout.placements[id];
    if (!placement) continue;
    if (placement.y > bottom) break;
    visibleIds.push(id);
  }
  return visibleIds;
}

export function reorderFileIdsForDraggedCard({
  orderedIds,
  draggedId,
  candidates,
  draggedCenterX,
  draggedCenterY,
}: {
  orderedIds: string[];
  draggedId: string;
  candidates: FileReorderCandidateRect[];
  draggedCenterX: number;
  draggedCenterY: number;
}) {
  const remainingIds = orderedIds.filter((id) => id !== draggedId);
  const candidateById = new Map(candidates.map((candidate) => [candidate.id, candidate]));
  if (remainingIds.some((id) => !candidateById.has(id))) return orderedIds;

  const visualCandidates = remainingIds
    .map((id) => candidateById.get(id)!)
    .sort((left, right) => left.top - right.top || left.left - right.left)
    .map((candidate) => ({
      ...candidate,
      centerX: candidate.left + (candidate.right - candidate.left) / 2,
    }));
  if (visualCandidates.length === 0) return [draggedId];

  const rows: FileReorderRow[] = [];
  visualCandidates.forEach((candidate) => {
    const currentRow = rows.at(-1);
    if (
      !currentRow
      || candidate.top >= currentRow.bottom
      || candidate.bottom <= currentRow.top
    ) {
      rows.push({
        top: candidate.top,
        bottom: candidate.bottom,
        items: [candidate],
      });
      return;
    }
    currentRow.top = Math.min(currentRow.top, candidate.top);
    currentRow.bottom = Math.max(currentRow.bottom, candidate.bottom);
    currentRow.items.push(candidate);
  });

  const rowDistance = (row: FileReorderRow) => {
    if (draggedCenterY < row.top) return row.top - draggedCenterY;
    if (draggedCenterY > row.bottom) return draggedCenterY - row.bottom;
    return 0;
  };
  const targetRow = rows.reduce((nearest, row) => (
    rowDistance(row) < rowDistance(nearest) ? row : nearest
  ));
  targetRow.items.sort((left, right) => left.left - right.left);

  const before = targetRow.items.find((candidate) => draggedCenterX <= candidate.centerX);
  const anchor = before ?? targetRow.items.at(-1)!;
  const visualIds = visualCandidates.map((candidate) => candidate.id);
  const anchorIndex = visualIds.indexOf(anchor.id);
  const insertionIndex = before ? anchorIndex : anchorIndex + 1;
  visualIds.splice(insertionIndex, 0, draggedId);
  return visualIds;
}
