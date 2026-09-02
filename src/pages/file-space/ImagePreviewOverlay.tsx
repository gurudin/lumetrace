import { invoke } from "@tauri-apps/api/core";
import { Minus, Plus, RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { TaskVersionTimelineRail, type PreviewTaskFileVersion } from "./TaskVersionTimelineRail";
import { VersionTimelineRegion, VersionTimelineToggle } from "./VersionTimelineToggle";
import { calculateWheelZoom, normalizeWheelDelta } from "./imagePreviewZoom";
import { shouldShowVersionTimelineByDefault } from "./versionTimelineVisibility";
import "./image-preview-overlay.css";

interface ImagePreviewRequest {
  fileId: string;
  name: string;
  source: string;
  currentSource: string;
  hasVersionHistory: boolean;
  versionCount: number;
}

interface ImageTaskFileVersion extends PreviewTaskFileVersion { mimeType: string | null; }
interface ImageTaskTimeline { currentVersionId: string; versions: ImageTaskFileVersion[]; }

interface DragState {
  pointerId: number;
  startX: number;
  startY: number;
  originX: number;
  originY: number;
}

const zoomMin = 0.25;
const zoomMax = 4;
const zoomStep = 0.25;

function clampZoom(value: number) {
  return Math.min(zoomMax, Math.max(zoomMin, value));
}

export function ImagePreviewOverlay() {
  const { t, i18n } = useTranslation();
  const copy = {
    preview: (name: string) => t("fileSpace.preview.image.dialog", { name }),
    zoomOut: t("fileSpace.preview.image.zoomOut"),
    zoomIn: t("fileSpace.preview.image.zoomIn"),
    reset: t("fileSpace.preview.image.reset"),
    close: t("fileSpace.preview.image.close"),
    loading: t("fileSpace.preview.image.loading"),
    loadError: t("fileSpace.preview.image.loadError"),
    hint: t("fileSpace.preview.image.hint"),
    versionHistory: t("fileSpace.preview.common.versionHistory"),
    showVersionHistory: t("fileSpace.preview.common.showVersionHistory"),
    hideVersionHistory: t("fileSpace.preview.common.hideVersionHistory"),
    versionCount: (count: number) => t("fileSpace.preview.common.versionCount", { count }),
    current: t("fileSpace.preview.common.current"),
    userEdit: t("fileSpace.preview.common.userEdit"),
    round: (count: number) => t("fileSpace.preview.common.round", { count }),
    timelineError: t("fileSpace.preview.common.timelineError"),
    retry: t("fileSpace.preview.common.retry"),
    historical: t("fileSpace.preview.common.historical"),
    timelineLoading: t("fileSpace.preview.common.loading"),
  };
  const [request, setRequest] = useState<ImagePreviewRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [wheelZooming, setWheelZooming] = useState(false);
  const [timeline, setTimeline] = useState<ImageTaskTimeline | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [timelineVisible, setTimelineVisible] = useState(false);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [versionLoading, setVersionLoading] = useState(false);
  const zoomRef = useRef(1);
  const pendingWheelDeltaRef = useRef(0);
  const wheelFrameRef = useRef<number | null>(null);
  const wheelIdleTimerRef = useRef<number | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const historicalSourceRef = useRef<string | null>(null);
  const presence = usePresence(open);
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const hasTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);
  const showTimeline = hasTimeline && timelineVisible;

  const cancelPendingWheelZoom = useCallback(() => {
    if (wheelFrameRef.current !== null) {
      window.cancelAnimationFrame(wheelFrameRef.current);
      wheelFrameRef.current = null;
    }
    if (wheelIdleTimerRef.current !== null) {
      window.clearTimeout(wheelIdleTimerRef.current);
      wheelIdleTimerRef.current = null;
    }
    pendingWheelDeltaRef.current = 0;
  }, []);

  const stopWheelGesture = useCallback(() => {
    cancelPendingWheelZoom();
    setWheelZooming(false);
  }, [cancelPendingWheelZoom]);

  const resetView = useCallback(() => {
    stopWheelGesture();
    zoomRef.current = 1;
    setZoom(1);
    setOffset({ x: 0, y: 0 });
  }, [stopWheelGesture]);

  const loadTimeline = useCallback(async (target: ImagePreviewRequest) => {
    if (!target.hasVersionHistory || target.versionCount <= 0) return;
    setTimelineLoading(true);
    setTimelineError(null);
    try {
      const loaded = await invoke<ImageTaskTimeline>("get_task_file_timeline", { fileId: target.fileId });
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
    } catch (error) {
      setTimelineError(error instanceof Error ? error.message : String(error));
    } finally {
      setTimelineLoading(false);
    }
  }, []);

  const selectVersion = useCallback(async (version: PreviewTaskFileVersion) => {
    if (!request || versionLoading || selectedVersionId === version.id) return;
    setVersionLoading(true);
    setFailed(false);
    setLoaded(false);
    resetView();
    try {
      let source = request.currentSource;
      if (!version.isCurrent) {
        const bytes = await invoke<number[]>("read_task_file_version", { fileId: request.fileId, versionId: version.id });
        const typedVersion = version as ImageTaskFileVersion;
        const nextSource = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: typedVersion.mimeType ?? "application/octet-stream" }));
        if (historicalSourceRef.current) URL.revokeObjectURL(historicalSourceRef.current);
        historicalSourceRef.current = nextSource;
        source = nextSource;
      } else if (historicalSourceRef.current) {
        URL.revokeObjectURL(historicalSourceRef.current);
        historicalSourceRef.current = null;
      }
      setSelectedVersionId(version.id);
      setRequest((current) => current ? { ...current, source } : current);
    } catch {
      setFailed(true);
    } finally {
      setVersionLoading(false);
    }
  }, [request, resetView, selectedVersionId, versionLoading]);

  const closePreview = useCallback(() => {
    stopWheelGesture();
    dragRef.current = null;
    setDragging(false);
    setOpen(false);
    window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, [stopWheelGesture]);

  const updateZoom = useCallback((value: number | ((currentZoom: number) => number)) => {
    const candidate = typeof value === "function" ? value(zoomRef.current) : value;
    const nextZoom = clampZoom(candidate);
    zoomRef.current = nextZoom;
    setZoom(nextZoom);
    if (nextZoom <= 1) setOffset({ x: 0, y: 0 });
  }, []);

  const updateZoomByStep = useCallback((step: number) => {
    stopWheelGesture();
    updateZoom((currentZoom) => currentZoom + step);
  }, [stopWheelGesture, updateZoom]);

  const queueWheelZoom = useCallback((deltaY: number, deltaMode: number) => {
    const normalizedDelta = normalizeWheelDelta(deltaY, deltaMode);
    if (normalizedDelta === 0) return;

    pendingWheelDeltaRef.current += normalizedDelta;
    setWheelZooming(true);

    if (wheelFrameRef.current === null) {
      wheelFrameRef.current = window.requestAnimationFrame(() => {
        wheelFrameRef.current = null;
        const frameDelta = pendingWheelDeltaRef.current;
        pendingWheelDeltaRef.current = 0;
        updateZoom((currentZoom) => calculateWheelZoom(currentZoom, frameDelta));
      });
    }

    if (wheelIdleTimerRef.current !== null) {
      window.clearTimeout(wheelIdleTimerRef.current);
    }
    wheelIdleTimerRef.current = window.setTimeout(() => {
      wheelIdleTimerRef.current = null;
      setWheelZooming(false);
    }, 120);
  }, [updateZoom]);

  useEffect(() => cancelPendingWheelZoom, [cancelPendingWheelZoom]);

  useEffect(() => {
    const imageCardFromTarget = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return null;
      const button = target.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
      const artwork = button?.querySelector<HTMLElement>(".file-space-file-art.has-preview");
      const image = artwork?.querySelector<HTMLImageElement>("img");
      if (!artwork || !button || !image) return null;
      return { button, image };
    };

    const openPreview = (button: HTMLButtonElement, image: HTMLImageElement) => {
      const source = image.currentSrc || image.src;
      if (!source) return;
      returnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const card = button.closest<HTMLElement>(".file-space-file-card");
      const fileId = card?.dataset.fileId;
      if (!fileId) return;
      const versionCount = Number(card.dataset.versionCount ?? "0");
      const target: ImagePreviewRequest = {
        fileId,
        name: button.title || image.alt,
        source,
        currentSource: source,
        hasVersionHistory: Number(card.dataset.versionCount ?? 0) > 0,
        versionCount: Number.isFinite(versionCount) ? Math.max(0, versionCount) : 0,
      };
      setRequest(target);
      setOpen(true);
      setLoaded(false);
      setFailed(false);
      resetView();
      setTimeline(null);
      setTimelineError(null);
      setTimelineVisible(shouldShowVersionTimelineByDefault(target.versionCount));
      setSelectedVersionId(null);
      void loadTimeline(target);
    };

    const handleImageDoubleClick = (event: MouseEvent) => {
      const card = imageCardFromTarget(event.target);
      if (!card || event.button !== 0) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      openPreview(card.button, card.image);
    };

    document.addEventListener("dblclick", handleImageDoubleClick, true);
    return () => {
      document.removeEventListener("dblclick", handleImageDoubleClick, true);
    };
  }, [loadTimeline, resetView]);

  useEffect(() => {
    if (!open) return undefined;
    window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") closePreview();
      if (event.key === "+" || event.key === "=") updateZoomByStep(zoomStep);
      if (event.key === "-") updateZoomByStep(-zoomStep);
      if (event.key === "0") resetView();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [closePreview, open, resetView, updateZoomByStep]);

  useEffect(() => {
    if (!presence.mounted) {
      setRequest(null);
      setTimeline(null);
      setTimelineVisible(false);
      if (historicalSourceRef.current) URL.revokeObjectURL(historicalSourceRef.current);
      historicalSourceRef.current = null;
    }
  }, [presence.mounted]);

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || zoom <= 1 || failed) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: offset.x,
      originY: offset.y,
    };
    setDragging(true);
  };

  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setOffset({
      x: drag.originX + event.clientX - drag.startX,
      y: drag.originY + event.clientY - drag.startY,
    });
  };

  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  if (!presence.mounted || !request) return null;

  return (
    <div
      className="file-image-preview"
      data-state={presence.state}
      role="dialog"
      aria-modal="true"
      aria-label={copy.preview(request.name)}
    >
      <header className="file-image-preview-toolbar">
        <div><strong title={request.name}>{request.name}</strong>{selectedVersion && !selectedVersion.isCurrent ? <small>{copy.historical}</small> : null}</div>
        <div className="file-image-preview-controls">
          <button
            type="button"
            disabled={!loaded || failed || zoom <= zoomMin}
            onClick={() => updateZoomByStep(-zoomStep)}
            aria-label={copy.zoomOut}
            title={copy.zoomOut}
          >
            <Minus size={17} />
          </button>
          <button
            className="file-image-preview-scale"
            type="button"
            disabled={!loaded || failed}
            onClick={resetView}
            aria-label={copy.reset}
            title={copy.reset}
          >
            {Math.round(zoom * 100)}%
          </button>
          <button
            type="button"
            disabled={!loaded || failed || zoom >= zoomMax}
            onClick={() => updateZoomByStep(zoomStep)}
            aria-label={copy.zoomIn}
            title={copy.zoomIn}
          >
            <Plus size={17} />
          </button>
          <button
            type="button"
            disabled={!loaded || failed || (zoom === 1 && offset.x === 0 && offset.y === 0)}
            onClick={resetView}
            aria-label={copy.reset}
            title={copy.reset}
          >
            <RotateCcw size={16} />
          </button>
          <span />
          <button
            ref={closeButtonRef}
            type="button"
            onClick={closePreview}
            aria-label={copy.close}
            title={copy.close}
          >
            <X size={18} />
          </button>
        </div>
      </header>

      <div className={`file-image-preview-workspace file-preview-version-workspace${showTimeline ? " has-version-timeline" : ""}`}>
        {hasTimeline ? (
          <VersionTimelineRegion visible={showTimeline}>
            <TaskVersionTimelineRail id="file-image-version-timeline" versions={timeline?.versions ?? null} versionCount={request.versionCount} selectedVersionId={selectedVersionId} loading={timelineLoading} error={timelineError} disabled={versionLoading} locale={i18n.resolvedLanguage ?? "en-US"} onSelect={(version) => void selectVersion(version)} onRetry={() => void loadTimeline(request)} tone="dark" copy={{ title: copy.versionHistory, count: copy.versionCount, loading: copy.timelineLoading, loadError: copy.timelineError, retry: copy.retry, current: copy.current, userEdit: copy.userEdit, round: copy.round }} />
          </VersionTimelineRegion>
        ) : null}
        {hasTimeline ? (
          <VersionTimelineToggle
            controlsId="file-image-version-timeline"
            visible={showTimeline}
            showLabel={copy.showVersionHistory}
            hideLabel={copy.hideVersionHistory}
            onToggle={() => setTimelineVisible((visible) => !visible)}
          />
        ) : null}
        <div
        className={`file-image-preview-canvas${zoom > 1 ? " is-zoomed" : ""}${dragging ? " is-dragging" : ""}${wheelZooming ? " is-wheel-zooming" : ""}`}
        onWheel={(event) => {
          if (!loaded || failed || event.deltaY === 0) return;
          event.preventDefault();
          queueWheelZoom(event.deltaY, event.deltaMode);
        }}
        onPointerDown={startDrag}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        {!loaded && !failed ? <span className="file-image-preview-loading" aria-label={copy.loading} /> : null}
        {failed ? <p role="alert">{copy.loadError}</p> : null}
        <div
          className="file-image-preview-stage"
          style={{ transform: `translate3d(${offset.x}px, ${offset.y}px, 0)` }}
        >
          <img
            src={request.source}
            alt={request.name}
            draggable={false}
            className={loaded && !failed ? "is-loaded" : ""}
            style={{ transform: `scale(${zoom})` }}
            onLoad={() => setLoaded(true)}
            onError={() => {
              setLoaded(false);
              setFailed(true);
            }}
          />
        </div>
        </div>
      </div>
      <p className="file-image-preview-hint">{copy.hint}</p>
    </div>
  );
}
