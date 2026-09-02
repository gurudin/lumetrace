const defaultZoomMin = 0.25;
const defaultZoomMax = 4;
const wheelDeltaLimit = 100;
const wheelZoomSensitivity = 0.00045;

export function normalizeWheelDelta(deltaY: number, deltaMode: number) {
  if (!Number.isFinite(deltaY)) return 0;
  const modeScale = deltaMode === 1 ? 16 : deltaMode === 2 ? 120 : 1;
  const scaledDelta = deltaY * modeScale;
  return Math.min(wheelDeltaLimit, Math.max(-wheelDeltaLimit, scaledDelta));
}

export function calculateWheelZoom(
  currentZoom: number,
  deltaY: number,
  deltaMode = 0,
  zoomMin = defaultZoomMin,
  zoomMax = defaultZoomMax,
) {
  if (!Number.isFinite(currentZoom)) return zoomMin;
  const normalizedDelta = normalizeWheelDelta(deltaY, deltaMode);
  const nextZoom = currentZoom * Math.exp(-normalizedDelta * wheelZoomSensitivity);
  return Math.min(zoomMax, Math.max(zoomMin, nextZoom));
}
