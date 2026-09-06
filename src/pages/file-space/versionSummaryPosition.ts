export function versionSummaryPosition(
  anchor: { left: number; right: number; top: number; bottom: number },
  viewport: { width: number; height: number },
  measuredHeight = 340,
) {
  const margin = 12;
  const gap = 8;
  const width = Math.min(320, viewport.width - margin * 2);
  const below = viewport.height - anchor.bottom - margin - gap;
  const above = anchor.top - margin - gap;
  const side = below >= Math.min(measuredHeight, 340) || below >= above ? "below" : "above";
  const maxHeight = Math.max(80, Math.min(340, side === "below" ? below : above));
  const height = Math.min(measuredHeight, maxHeight);
  const left = Math.max(margin, Math.min(anchor.right - width, viewport.width - width - margin));
  const top = Math.max(margin, Math.min(side === "below" ? anchor.bottom + gap : anchor.top - gap - height, viewport.height - height - margin));
  const arrow = Math.max(16, Math.min(width - 16, (anchor.left + anchor.right) / 2 - left));
  return { left, top, width, maxHeight, arrow, side };
}
