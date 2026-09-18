export interface SearchVisualRect { x: number; y: number; width: number; height: number }
export interface SearchVisualSection {
  text: string;
  pageNumber: number | null;
  rectangles?: SearchVisualRect[];
}

export function visualPreviewKind(name: string): "image" | "pdf" | null {
  if (/\.pdf$/i.test(name)) return "pdf";
  return /\.(png|jpe?g|webp|gif|bmp|tiff?|heic|heif)$/i.test(name) ? "image" : null;
}

/** Join adjacent character boxes into a readable word/phrase highlight. */
export function mergeVisualRectangles(rectangles: SearchVisualRect[]) {
  const result: SearchVisualRect[] = [];
  for (const rect of rectangles) {
    if (![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite)
      || rect.x < 0 || rect.y < 0 || rect.width <= 0 || rect.height <= 0
      || rect.x + rect.width > 1.001 || rect.y + rect.height > 1.001) continue;
    const last = result.at(-1);
    if (last && Math.abs(last.y - rect.y) < Math.max(last.height, rect.height) * 0.4
      && rect.x >= last.x && rect.x <= last.x + last.width + Math.max(last.height, rect.height) * 0.35) {
      const bottom = Math.max(last.y + last.height, rect.y + rect.height);
      last.width = Math.max(last.x + last.width, rect.x + rect.width) - last.x;
      last.y = Math.min(last.y, rect.y);
      last.height = bottom - last.y;
    } else result.push({ ...rect });
  }
  return result;
}
