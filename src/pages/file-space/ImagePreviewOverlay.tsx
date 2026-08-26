import { invoke } from "@tauri-apps/api/core";
import { Minus, Plus, RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { TaskVersionTimelineRail, type PreviewTaskFileVersion } from "./TaskVersionTimelineRail";
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
  const { i18n } = useTranslation();
  const copy = i18n.resolvedLanguage?.startsWith("zh") ? {
    preview: (name: string) => `预览图片：${name}`,
    zoomOut: "缩小图片",
    zoomIn: "放大图片",
    reset: "恢复适应屏幕",
    close: "关闭图片预览",
    loading: "正在载入图片",
    loadError: "无法载入这张图片。",
    hint: "滚轮缩放 · 放大后拖动查看 · Esc 关闭",
    versionHistory: "版本记录",
    versionCount: (count: number) => `共 ${count} 个版本`,
    current: "当前版本",
    userEdit: "用户修改",
    round: (count: number) => `第 ${count} 轮`,
    timelineError: "无法读取版本记录",
    retry: "重试",
    historical: "历史版本 · 只读",
  } : {
    preview: (name: string) => `Preview image: ${name}`,
    zoomOut: "Zoom out",
    zoomIn: "Zoom in",
    reset: "Fit to screen",
    close: "Close image preview",
    loading: "Loading image",
    loadError: "Unable to load this image.",
    hint: "Scroll to zoom · Drag when zoomed · Esc to close",
    versionHistory: "Version history",
    versionCount: (count: number) => `${count} version${count === 1 ? "" : "s"}`,
    current: "Current",
    userEdit: "User edit",
    round: (count: number) => `Round ${count}`,
    timelineError: "Unable to load version history",
    retry: "Retry",
    historical: "Historical version · Read only",
  };
  const [request, setRequest] = useState<ImagePreviewRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [timeline, setTimeline] = useState<ImageTaskTimeline | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [versionLoading, setVersionLoading] = useState(false);
  const dragRef = useRef<DragState | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const historicalSourceRef = useRef<string | null>(null);
  const presence = usePresence(open);
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const showTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);

  const resetView = useCallback(() => {
    setZoom(1);
    setOffset({ x: 0, y: 0 });
  }, []);

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
    dragRef.current = null;
    setDragging(false);
    setOpen(false);
    window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, []);

  const updateZoom = useCallback((value: number) => {
    const nextZoom = clampZoom(value);
    setZoom(nextZoom);
    if (nextZoom <= 1) setOffset({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    const imageCardFromTarget = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return null;
      const artwork = target.closest<HTMLElement>(".file-space-file-art.has-preview");
      const button = artwork?.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
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
      if (event.key === "+" || event.key === "=") updateZoom(zoom + zoomStep);
      if (event.key === "-") updateZoom(zoom - zoomStep);
      if (event.key === "0") resetView();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [closePreview, open, resetView, updateZoom, zoom]);

  useEffect(() => {
    if (!presence.mounted) {
      setRequest(null);
      setTimeline(null);
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
            onClick={() => updateZoom(zoom - zoomStep)}
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
            onClick={() => updateZoom(zoom + zoomStep)}
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

      <div className={`file-image-preview-workspace${showTimeline ? " has-version-timeline" : ""}`}>
        {showTimeline ? <TaskVersionTimelineRail versions={timeline?.versions ?? null} versionCount={request.versionCount} selectedVersionId={selectedVersionId} loading={timelineLoading} error={timelineError} disabled={versionLoading} locale={i18n.resolvedLanguage?.startsWith("zh") ? "zh-CN" : "en-US"} onSelect={(version) => void selectVersion(version)} onRetry={() => void loadTimeline(request)} tone="dark" copy={{ title: copy.versionHistory, count: copy.versionCount, loading: copy.loading, loadError: copy.timelineError, retry: copy.retry, current: copy.current, userEdit: copy.userEdit, round: copy.round }} /> : null}
        <div
        className={`file-image-preview-canvas${zoom > 1 ? " is-zoomed" : ""}${dragging ? " is-dragging" : ""}`}
        onWheel={(event) => {
          event.preventDefault();
          updateZoom(zoom + (event.deltaY < 0 ? zoomStep : -zoomStep));
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
