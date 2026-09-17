import { convertFileSrc } from "@tauri-apps/api/core";
import { LoaderCircle } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import workerSrc from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { useWorkspaceExtension } from "../../shared/extensions/ApplicationExtension";
import { mergeVisualRectangles, type SearchVisualSection } from "./searchVisualGeometry";

export function SearchVisualPreview({ file, kind, sections, activeIndex, ready }: {
  file: { id: string; name: string; updatedAt: number };
  kind: "image" | "pdf";
  sections: SearchVisualSection[];
  activeIndex: number;
  ready: boolean;
}) {
  const invoke = useWorkspaceInvoke();
  const source = useWorkspaceExtension()?.source ?? null;
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [pageNumber, setPageNumber] = useState(1);
  const [renderedPage, setRenderedPage] = useState<number | null>(null);
  const [width, setWidth] = useState(600);
  const activeSection = sections[activeIndex];

  useEffect(() => {
    if (ready) setPageNumber(activeSection?.pageNumber ?? 1);
  }, [ready, activeSection?.pageNumber]);

  useEffect(() => {
    const surface = surfaceRef.current;
    if (!surface) return;
    const observer = new ResizeObserver(() => setWidth(Math.max(1, surface.clientWidth)));
    observer.observe(surface);
    return () => observer.disconnect();
  }, []);

  // Load only when file/revision/source changes, never for every query or hit.
  useEffect(() => {
    let cancelled = false;
    let task: PDFDocumentLoadingTask | undefined;
    let image: HTMLImageElement | undefined;
    setLoading(true); setFailed(false); setPdf(null); setRenderedPage(null);
    void (async () => {
      try {
        if (kind === "pdf") {
          const bytes = await invoke<ArrayBuffer | number[]>("read_file_space_pdf", { fileId: file.id });
          if (cancelled) return;
          const pdfjs = await import("pdfjs-dist");
          if (cancelled) return;
          pdfjs.GlobalWorkerOptions.workerSrc = workerSrc;
          task = pdfjs.getDocument({ data: new Uint8Array(bytes as ArrayBuffer), isEvalSupported: false });
          const document = await task.promise;
          if (cancelled) { await document.destroy(); return; }
          setPdf(document);
        } else {
          const resolved = source?.resolveImagePreview
            ? await source.resolveImagePreview(file.id, file.updatedAt) : null;
          if (cancelled) return;
          const url = source
            ? resolved?.source ?? source.imagePreviewUrl?.(file.id, file.updatedAt, "detail")
            : `${convertFileSrc(file.id, "lumetrace-file-preview")}?revision=${file.updatedAt}`;
          if (!url) throw new Error("Image preview unavailable");
          image = new Image();
          image.onload = () => {
            if (cancelled) return;
            const canvas = canvasRef.current;
            if (!canvas) return;
            const scale = Math.min(1, 2400 / Math.max(image!.naturalWidth, image!.naturalHeight));
            canvas.width = Math.max(1, Math.round(image!.naturalWidth * scale));
            canvas.height = Math.max(1, Math.round(image!.naturalHeight * scale));
            const context = canvas.getContext("2d");
            if (!context) { setFailed(true); setLoading(false); return; }
            // Freeze animated containers at their first frame, matching OCR.
            context.drawImage(image!, 0, 0, canvas.width, canvas.height);
            setRenderedPage(1); setLoading(false);
          };
          image.onerror = () => { if (!cancelled) { setFailed(true); setLoading(false); } };
          image.src = url;
        }
      } catch {
        if (!cancelled) { setFailed(true); setLoading(false); }
      }
    })();
    return () => {
      cancelled = true;
      if (image) { image.onload = null; image.onerror = null; image.src = ""; }
      void task?.destroy();
    };
  }, [file.id, file.updatedAt, kind, source, invoke, attempt]);

  useEffect(() => {
    if (!pdf) return;
    let cancelled = false;
    let task: RenderTask | undefined;
    setLoading(true); setFailed(false); setRenderedPage(null);
    void (async () => {
      try {
        if (pageNumber > pdf.numPages) throw new Error("Indexed PDF page no longer exists");
        const page = await pdf.getPage(pageNumber);
        if (cancelled) return;
        const canvas = canvasRef.current;
        if (!canvas) return;
        const base = page.getViewport({ scale: 1 });
        const scale = Math.min(width * Math.min(devicePixelRatio || 1, 2) / base.width, 2400 / Math.max(base.width, base.height));
        const viewport = page.getViewport({ scale });
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        task = page.render({ canvas, viewport });
        await task.promise;
        if (!cancelled) { setRenderedPage(pageNumber); setLoading(false); }
      } catch {
        if (!cancelled) { setFailed(true); setLoading(false); }
      }
    })();
    return () => { cancelled = true; task?.cancel(); };
  }, [pdf, pageNumber, width]);

  const showBoxes = ready && !loading && !failed && renderedPage === pageNumber;
  useEffect(() => {
    if (!showBoxes) return;
    const surface = surfaceRef.current;
    const scroller = surface?.closest(".file-space-global-search-preview-body");
    const rect = mergeVisualRectangles(activeSection?.rectangles ?? [])[0];
    if (!surface || !scroller || !rect) return;
    const top = surface.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
    const hitTop = top + rect.y * surface.clientHeight;
    const hitBottom = hitTop + rect.height * surface.clientHeight;
    if (hitTop < scroller.scrollTop + 12 || hitBottom > scroller.scrollTop + scroller.clientHeight - 12) {
      scroller.scrollTo({ top: Math.max(0, hitTop - scroller.clientHeight * 0.3), behavior: "instant" });
    }
  }, [showBoxes, activeIndex, activeSection, renderedPage]);

  return <div className="search-visual-preview" aria-busy={loading}>
    {kind === "pdf" && !failed ? <div className="file-space-global-search-preview-location">
      {t("fileSpace.globalSearch.pageLocation", { page: pageNumber })}
    </div> : null}
    {(loading || failed) ? <div className="file-space-global-search-preview-state" role={failed ? "alert" : "status"}>
      {loading ? <LoaderCircle className="is-spinning" size={20} /> : null}
      <span>{t(failed ? "fileSpace.globalSearch.previewError" : "fileSpace.globalSearch.loadingPreview")}</span>
      {failed ? <button type="button" onClick={() => setAttempt(value => value + 1)}>{t("fileSpace.preview.common.retry")}</button> : null}
    </div> : null}
    <div ref={surfaceRef} className="search-visual-surface" style={{ visibility: loading || failed ? "hidden" : "visible", height: loading || failed ? 0 : undefined }}>
      <canvas ref={canvasRef} role="img" aria-label={file.name} aria-description={activeSection?.text} />
      <svg className="search-visual-highlights" viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden="true">
        {showBoxes ? sections.flatMap((section, index) => (section.pageNumber ?? 1) === pageNumber
          ? mergeVisualRectangles(section.rectangles ?? []).map((rect, n) => <rect key={`${index}:${n}`}
              {...rect} rx={0.003} className={index === activeIndex ? "is-current" : undefined} />)
          : []) : null}
      </svg>
    </div>
  </div>;
}
