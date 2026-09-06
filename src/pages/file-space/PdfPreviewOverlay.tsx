import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  LoaderCircle,
  Maximize2,
  Minus,
  Plus,
  X,
} from "lucide-react";
import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import workerSrc from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { TaskVersionTimelineRail, type PreviewTaskFileVersion } from "./TaskVersionTimelineRail";
import { useVersionAnnotationUpdates } from "./VersionAnnotation";
import { VersionTimelineRegion, VersionTimelineToggle } from "./VersionTimelineToggle";
import { shouldShowVersionTimelineByDefault } from "./versionTimelineVisibility";
import { PreviewFileHeading } from "./PreviewFileHeading";
import "./pdf-preview-overlay.css";

interface PdfPreviewRequest {
  fileId: string;
  name: string;
  hasVersionHistory: boolean;
  versionCount: number;
}

interface PdfTaskTimeline { workspaceId?: string; fileId: string; currentVersionId: string; versions: PreviewTaskFileVersion[]; }

interface PdfPageCanvasProps {
  document: PDFDocumentProxy;
  pageNumber: number;
  scale: number;
  scrollRoot: HTMLElement | null;
}

const zoomMin = 0.5;
const zoomMax = 2.5;
const zoomStep = 0.15;

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function isPdfName(name: string) {
  return /\.pdf$/i.test(name.trim());
}

function clampZoom(value: number) {
  return Math.min(zoomMax, Math.max(zoomMin, Number(value.toFixed(2))));
}

function createPdfVisualFixture() {
  const pageObjects = [3, 5, 7];
  const contentObjects = [4, 6, 9];
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pageObjects.map((id) => `${id} 0 R`).join(" ")}] /Count 3 >>`,
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> >> /Contents 4 0 R >>",
    "",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> >> /Contents 6 0 R >>",
    "",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> >> /Contents 9 0 R >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    "",
  ];
  [1, 2, 3].forEach((pageNumber, index) => {
    const content = `BT /F1 23 Tf 72 716 Td (Lume Trace PDF Preview) Tj 0 -36 Td /F1 14 Tf (Page ${pageNumber} of 3) Tj 0 -48 Td /F1 11 Tf (Multi-page scroll, page navigation and zoom fixture.) Tj ET`;
    objects[contentObjects[index] - 1] = `<< /Length ${content.length} >>\nstream\n${content}\nendstream`;
  });
  let output = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((body, index) => {
    offsets.push(output.length);
    output += `${index + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xrefOffset = output.length;
  output += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  offsets.slice(1).forEach((offset) => {
    output += `${String(offset).padStart(10, "0")} 00000 n \n`;
  });
  output += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return new TextEncoder().encode(output);
}

function PdfPageCanvas({ document, pageNumber, scale, scrollRoot }: PdfPageCanvasProps) {
  const holderRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderTaskRef = useRef<RenderTask | null>(null);
  const [nearViewport, setNearViewport] = useState(pageNumber <= 2);
  const [baseSize, setBaseSize] = useState({ width: 612, height: 792 });
  const [rendered, setRendered] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const holder = holderRef.current;
    if (!holder || !scrollRoot || nearViewport) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setNearViewport(true);
          observer.disconnect();
        }
      },
      { root: scrollRoot, rootMargin: "900px 0px" },
    );
    observer.observe(holder);
    return () => observer.disconnect();
  }, [nearViewport, scrollRoot]);

  useEffect(() => {
    if (!nearViewport) return undefined;
    let cancelled = false;
    const render = async () => {
      try {
        const page = await document.getPage(pageNumber);
        if (cancelled) return;
        const displayViewport = page.getViewport({ scale });
        const unscaledViewport = page.getViewport({ scale: 1 });
        setBaseSize({ width: unscaledViewport.width, height: unscaledViewport.height });
        const outputScale = Math.min(window.devicePixelRatio || 1, 2);
        const renderViewport = page.getViewport({ scale: scale * outputScale });
        const canvas = canvasRef.current;
        if (!canvas || cancelled) return;
        canvas.width = Math.ceil(renderViewport.width);
        canvas.height = Math.ceil(renderViewport.height);
        canvas.style.width = `${Math.ceil(displayViewport.width)}px`;
        canvas.style.height = `${Math.ceil(displayViewport.height)}px`;
        setRendered(false);
        setFailed(false);
        renderTaskRef.current?.cancel();
        const task = page.render({ canvas, viewport: renderViewport });
        renderTaskRef.current = task;
        await task.promise;
        if (!cancelled) setRendered(true);
      } catch (error) {
        if (!cancelled && !(error instanceof Error && error.name === "RenderingCancelledException")) {
          setFailed(true);
        }
      }
    };
    void render();
    return () => {
      cancelled = true;
      renderTaskRef.current?.cancel();
      renderTaskRef.current = null;
    };
  }, [document, nearViewport, pageNumber, scale]);

  return (
    <section
      ref={holderRef}
      className="file-pdf-preview-page"
      data-page={pageNumber}
      aria-label={`${pageNumber}`}
      style={{ width: `${baseSize.width * scale}px`, height: `${baseSize.height * scale}px` }}
    >
      {!rendered && !failed ? <span className="file-pdf-preview-page-loading" aria-hidden="true" /> : null}
      {failed ? <AlertTriangle className="file-pdf-preview-page-error" size={24} aria-hidden="true" /> : null}
      <canvas ref={canvasRef} className={rendered ? "is-rendered" : ""} />
    </section>
  );
}

export function PdfPreviewOverlay() {
  const { t, i18n } = useTranslation();
  const copy = {
    dialog: (name: string) => t("fileSpace.preview.pdf.dialog", { name }),
    previous: t("fileSpace.preview.pdf.previous"),
    next: t("fileSpace.preview.pdf.next"),
    page: t("fileSpace.preview.pdf.page"),
    zoomOut: t("fileSpace.preview.pdf.zoomOut"),
    zoomIn: t("fileSpace.preview.pdf.zoomIn"),
    fitWidth: t("fileSpace.preview.pdf.fitWidth"),
    close: t("fileSpace.preview.pdf.close"),
    loading: t("fileSpace.preview.pdf.loading"),
    loadError: t("fileSpace.preview.pdf.loadError"),
    retry: t("fileSpace.preview.pdf.retry"),
    desktopOnly: t("fileSpace.preview.pdf.desktopOnly"),
    versionHistory: t("fileSpace.preview.common.versionHistory"),
    showVersionHistory: t("fileSpace.preview.common.showVersionHistory"),
    hideVersionHistory: t("fileSpace.preview.common.hideVersionHistory"),
    versionCount: (count: number) => t("fileSpace.preview.common.versionCount", { count }),
    current: t("fileSpace.preview.common.current"),
    userEdit: t("fileSpace.preview.common.userEdit"),
    round: (count: number) => t("fileSpace.preview.common.round", { count }),
    timelineError: t("fileSpace.preview.common.timelineError"),
    historical: t("fileSpace.preview.common.historical"),
    timelineLoading: t("fileSpace.preview.common.loading"),
  };
  const [request, setRequest] = useState<PdfPreviewRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [pdfDocument, setPdfDocument] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [currentPage, setCurrentPage] = useState(1);
  const [pageDraft, setPageDraft] = useState("1");
  const [zoom, setZoom] = useState(1);
  const [fitScale, setFitScale] = useState(1);
  const [scrollRoot, setScrollRoot] = useState<HTMLElement | null>(null);
  const [timeline, setTimeline] = useState<PdfTaskTimeline | null>(null);
  useVersionAnnotationUpdates(setTimeline);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [timelineVisible, setTimelineVisible] = useState(false);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const scrollRef = useRef<HTMLElement>(null);
  const firstPageWidthRef = useRef(612);
  const loadingTaskRef = useRef<PDFDocumentLoadingTask | null>(null);
  const documentRef = useRef<PDFDocumentProxy | null>(null);
  const loadSequenceRef = useRef(0);
  const fitScaleUpdateRef = useRef<() => void>(() => undefined);
  const timelineMotionRef = useRef(false);
  const timelineMotionTimerRef = useRef<number | null>(null);
  const presence = usePresence(open);
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const hasTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);
  const showTimeline = hasTimeline && timelineVisible;

  const finishTimelineMotion = useCallback(() => {
    if (timelineMotionTimerRef.current !== null) {
      window.clearTimeout(timelineMotionTimerRef.current);
      timelineMotionTimerRef.current = null;
    }
    timelineMotionRef.current = false;
    window.requestAnimationFrame(() => fitScaleUpdateRef.current());
  }, []);

  const toggleTimeline = useCallback(() => {
    setTimelineVisible((visible) => !visible);
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      window.requestAnimationFrame(() => fitScaleUpdateRef.current());
      return;
    }
    timelineMotionRef.current = true;
    if (timelineMotionTimerRef.current !== null) window.clearTimeout(timelineMotionTimerRef.current);
    timelineMotionTimerRef.current = window.setTimeout(finishTimelineMotion, 220);
  }, [finishTimelineMotion]);

  const updateDocument = useCallback((nextDocument: PDFDocumentProxy | null) => {
    documentRef.current = nextDocument;
    setPdfDocument(nextDocument);
  }, []);

  const destroyDocument = useCallback(() => {
    loadSequenceRef.current += 1;
    loadingTaskRef.current?.destroy();
    loadingTaskRef.current = null;
    void documentRef.current?.destroy();
    updateDocument(null);
  }, [updateDocument]);

  const loadPdf = useCallback(async (target: PdfPreviewRequest, version?: PreviewTaskFileVersion) => {
    destroyDocument();
    const loadSequence = loadSequenceRef.current;
    setLoading(true);
    setLoadError(null);
    setCurrentPage(1);
    setPageDraft("1");
    setZoom(1);
    try {
      const visualFixtureEnabled = import.meta.env.DEV
        && new URLSearchParams(window.location.search).has("fileSpacePreview")
        && target.fileId.startsWith("fixture-file-");
      if (!isTauri() && !visualFixtureEnabled) throw new Error(copy.desktopOnly);
      const bytes = visualFixtureEnabled
        ? createPdfVisualFixture()
        : version && !version.isCurrent
          ? await invoke<number[]>("read_task_file_version", { fileId: target.fileId, versionId: version.id })
          : await invoke<ArrayBuffer>("read_file_space_pdf", { fileId: target.fileId });
      if (loadSequence !== loadSequenceRef.current) return;
      const pdfjs = await import("pdfjs-dist");
      pdfjs.GlobalWorkerOptions.workerSrc = workerSrc;
      const task = pdfjs.getDocument({ data: bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes), isEvalSupported: false });
      loadingTaskRef.current = task;
      const loadedDocument = await task.promise;
      if (loadSequence !== loadSequenceRef.current) {
        void loadedDocument.destroy();
        return;
      }
      const firstPage = await loadedDocument.getPage(1);
      firstPageWidthRef.current = firstPage.getViewport({ scale: 1 }).width;
      loadingTaskRef.current = null;
      updateDocument(loadedDocument);
    } catch (error) {
      if (loadSequence === loadSequenceRef.current) setLoadError(errorText(error));
    } finally {
      if (loadSequence === loadSequenceRef.current) setLoading(false);
    }
  }, [copy.desktopOnly, destroyDocument, updateDocument]);

  const loadTimeline = useCallback(async (target: PdfPreviewRequest) => {
    if (!target.hasVersionHistory || target.versionCount <= 0) return;
    setTimelineLoading(true);
    setTimelineError(null);
    try {
      const loaded = await invoke<PdfTaskTimeline>("get_task_file_timeline", { fileId: target.fileId });
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
    } catch (error) {
      setTimelineError(errorText(error));
    } finally {
      setTimelineLoading(false);
    }
  }, []);

  const selectVersion = useCallback(async (version: PreviewTaskFileVersion) => {
    if (!request || loading || selectedVersionId === version.id) return;
    setSelectedVersionId(version.id);
    await loadPdf(request, version);
  }, [loadPdf, loading, request, selectedVersionId]);

  const closePreview = useCallback(() => {
    setOpen(false);
    window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, []);

  const scrollToPage = useCallback((page: number) => {
    if (!pdfDocument) return;
    const nextPage = Math.min(pdfDocument.numPages, Math.max(1, Math.round(page)));
    scrollRef.current
      ?.querySelector<HTMLElement>(`[data-page="${nextPage}"]`)
      ?.scrollIntoView({ block: "start", behavior: "smooth" });
    setCurrentPage(nextPage);
    setPageDraft(String(nextPage));
  }, [pdfDocument]);

  useEffect(() => {
    const pdfCardFromTarget = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return null;
      const button = target.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
      const card = button?.closest<HTMLElement>(".file-space-file-card");
      const fileId = button?.dataset.fileId ?? card?.dataset.fileId;
      const name = button?.title ?? "";
      if (!button || !card || !fileId || !isPdfName(name)) return null;
      const versionCount = Number(card.dataset.versionCount ?? "0");
      return {
        button,
        request: {
          fileId,
          name,
          hasVersionHistory: Number(card.dataset.versionCount ?? 0) > 0,
          versionCount: Number.isFinite(versionCount) ? Math.max(0, versionCount) : 0,
        } satisfies PdfPreviewRequest,
      };
    };

    const handleDoubleClick = (event: MouseEvent) => {
      const card = pdfCardFromTarget(event.target);
      if (!card || event.button !== 0) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      returnFocusRef.current = card.button;
      setRequest(card.request);
      setTimeline(null);
      setTimelineError(null);
      setTimelineVisible(shouldShowVersionTimelineByDefault());
      setSelectedVersionId(null);
      setOpen(true);
      void loadPdf(card.request);
      void loadTimeline(card.request);
    };

    document.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      document.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, [loadPdf, loadTimeline]);

  useEffect(() => {
    if (!open) return undefined;
    window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closePreview();
      } else if (event.key === "PageUp") {
        event.preventDefault();
        scrollToPage(currentPage - 1);
      } else if (event.key === "PageDown") {
        event.preventDefault();
        scrollToPage(currentPage + 1);
      } else if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        setZoom((value) => clampZoom(value + zoomStep));
      } else if (event.key === "-") {
        event.preventDefault();
        setZoom((value) => clampZoom(value - zoomStep));
      } else if (event.key === "0") {
        event.preventDefault();
        setZoom(1);
      } else if (event.key === "Tab") {
        const focusable = Array.from(document.querySelectorAll<HTMLElement>(
          '.file-pdf-preview button:not(:disabled), .file-pdf-preview input:not(:disabled)',
        )).filter((element) => element.offsetParent !== null);
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [closePreview, currentPage, open, scrollToPage]);

  useLayoutEffect(() => {
    if (!presence.mounted) return undefined;
    const scroller = scrollRef.current;
    if (!scroller) return undefined;
    setScrollRoot(scroller);
    const updateFitScale = () => {
      if (timelineMotionRef.current) return;
      const availableWidth = Math.max(240, scroller.clientWidth - 64);
      setFitScale(Math.min(2, availableWidth / firstPageWidthRef.current));
    };
    fitScaleUpdateRef.current = updateFitScale;
    updateFitScale();
    const observer = new ResizeObserver(updateFitScale);
    observer.observe(scroller);
    return () => {
      observer.disconnect();
      if (fitScaleUpdateRef.current === updateFitScale) fitScaleUpdateRef.current = () => undefined;
    };
  }, [pdfDocument, presence.mounted]);

  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller || !pdfDocument) return undefined;
    let frame = 0;
    const updateCurrentPage = () => {
      frame = 0;
      const targetY = scroller.getBoundingClientRect().top + Math.min(160, scroller.clientHeight * 0.22);
      let closestPage = 1;
      let closestDistance = Number.POSITIVE_INFINITY;
      scroller.querySelectorAll<HTMLElement>("[data-page]").forEach((page) => {
        const bounds = page.getBoundingClientRect();
        const distance = bounds.top <= targetY && bounds.bottom >= targetY
          ? 0
          : Math.min(Math.abs(bounds.top - targetY), Math.abs(bounds.bottom - targetY));
        if (distance < closestDistance) {
          closestDistance = distance;
          closestPage = Number(page.dataset.page) || 1;
        }
      });
      setCurrentPage(closestPage);
      setPageDraft(String(closestPage));
    };
    const handleScroll = () => {
      if (!frame) frame = window.requestAnimationFrame(updateCurrentPage);
    };
    scroller.addEventListener("scroll", handleScroll, { passive: true });
    updateCurrentPage();
    return () => {
      scroller.removeEventListener("scroll", handleScroll);
      if (frame) window.cancelAnimationFrame(frame);
    };
  }, [pdfDocument]);

  useEffect(() => {
    if (!presence.mounted) {
      destroyDocument();
      setRequest(null);
      setScrollRoot(null);
      setTimeline(null);
      setTimelineVisible(false);
    }
  }, [destroyDocument, presence.mounted]);

  useEffect(() => () => {
    if (timelineMotionTimerRef.current !== null) window.clearTimeout(timelineMotionTimerRef.current);
    timelineMotionRef.current = false;
    destroyDocument();
  }, [destroyDocument]);

  if (!presence.mounted || !request) return null;

  const pageCount = pdfDocument?.numPages ?? 0;
  const scale = fitScale * zoom;

  return (
    <div
      className="file-pdf-preview"
      data-state={presence.state}
      role="dialog"
      aria-modal="true"
      aria-label={copy.dialog(request.name)}
    >
      <header className="file-pdf-preview-toolbar">
        <PreviewFileHeading name={request.name} versionCount={timeline?.versions.length ?? request.versionCount}>
          {selectedVersion && !selectedVersion.isCurrent ? <small>{copy.historical}</small> : null}
        </PreviewFileHeading>
        <div className="file-pdf-preview-controls">
          <button type="button" disabled={!pdfDocument || currentPage <= 1} onClick={() => scrollToPage(currentPage - 1)} title={copy.previous} aria-label={copy.previous}>
            <ChevronLeft size={17} />
          </button>
          <form
            className="file-pdf-preview-page-control"
            onSubmit={(event) => {
              event.preventDefault();
              scrollToPage(Number(pageDraft));
            }}
          >
            <input
              type="text"
              inputMode="numeric"
              value={pageDraft}
              disabled={!pdfDocument}
              onChange={(event) => setPageDraft(event.target.value.replace(/\D/g, ""))}
              onBlur={() => setPageDraft(String(currentPage))}
              aria-label={copy.page}
            />
            <span>/ {pageCount || "–"}</span>
          </form>
          <button type="button" disabled={!pdfDocument || currentPage >= pageCount} onClick={() => scrollToPage(currentPage + 1)} title={copy.next} aria-label={copy.next}>
            <ChevronRight size={17} />
          </button>
          <span className="file-pdf-preview-divider" />
          <button type="button" disabled={!pdfDocument || zoom <= zoomMin} onClick={() => setZoom((value) => clampZoom(value - zoomStep))} title={copy.zoomOut} aria-label={copy.zoomOut}>
            <Minus size={17} />
          </button>
          <button className="file-pdf-preview-scale" type="button" disabled={!pdfDocument} onClick={() => setZoom(1)} title={copy.fitWidth} aria-label={copy.fitWidth}>
            {Math.round(zoom * 100)}%
          </button>
          <button type="button" disabled={!pdfDocument || zoom >= zoomMax} onClick={() => setZoom((value) => clampZoom(value + zoomStep))} title={copy.zoomIn} aria-label={copy.zoomIn}>
            <Plus size={17} />
          </button>
          <button className="file-pdf-preview-fit" type="button" disabled={!pdfDocument} onClick={() => setZoom(1)} title={copy.fitWidth} aria-label={copy.fitWidth}>
            <Maximize2 size={15} /><span>{copy.fitWidth}</span>
          </button>
          <span className="file-pdf-preview-divider" />
          <button ref={closeButtonRef} type="button" onClick={closePreview} title={copy.close} aria-label={copy.close}>
            <X size={18} />
          </button>
        </div>
      </header>

      <div
        className={`file-pdf-preview-workspace file-preview-version-workspace${showTimeline ? " has-version-timeline" : ""}`}
        onTransitionEnd={(event) => {
          if (event.target === event.currentTarget && event.propertyName === "grid-template-columns") {
            finishTimelineMotion();
          }
        }}
      >
        {hasTimeline ? (
          <VersionTimelineRegion visible={showTimeline}>
            <TaskVersionTimelineRail workspaceId={timeline?.workspaceId} fileId={request.fileId} id="file-pdf-version-timeline" versions={timeline?.versions ?? null} versionCount={request.versionCount} selectedVersionId={selectedVersionId} loading={timelineLoading} error={timelineError} disabled={loading} locale={i18n.resolvedLanguage ?? "en-US"} onSelect={(version) => void selectVersion(version)} onRetry={() => void loadTimeline(request)} copy={{ title: copy.versionHistory, count: copy.versionCount, loading: copy.timelineLoading, loadError: copy.timelineError, retry: copy.retry, current: copy.current, userEdit: copy.userEdit, round: copy.round }} />
          </VersionTimelineRegion>
        ) : null}
        {hasTimeline ? (
          <VersionTimelineToggle
            controlsId="file-pdf-version-timeline"
            visible={showTimeline}
            showLabel={copy.showVersionHistory}
            hideLabel={copy.hideVersionHistory}
            onToggle={toggleTimeline}
          />
        ) : null}
        <main ref={scrollRef} className="file-pdf-preview-body">
        {loading ? (
          <div className="file-pdf-preview-state" role="status">
            <LoaderCircle className="is-spinning" size={24} />
            <span>{copy.loading}</span>
          </div>
        ) : loadError ? (
          <div className="file-pdf-preview-state is-error" role="alert">
            <AlertTriangle size={23} />
            <strong>{copy.loadError}</strong>
            <span>{loadError}</span>
            <button type="button" onClick={() => void loadPdf(request)}>{copy.retry}</button>
          </div>
        ) : pdfDocument ? (
          <div className="file-pdf-preview-pages">
            {Array.from({ length: pdfDocument.numPages }, (_, index) => (
              <PdfPageCanvas
                key={index + 1}
                document={pdfDocument}
                pageNumber={index + 1}
                scale={scale}
                scrollRoot={scrollRoot}
              />
            ))}
          </div>
        ) : null}
        </main>
      </div>
    </div>
  );
}
