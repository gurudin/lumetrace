import { Clock3, LoaderCircle, Star } from "lucide-react";
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useVersionAnnotationUpdates } from "./VersionAnnotation";
import { VersionName } from "./InitialVersionHint";
import { versionSummaryPosition } from "./versionSummaryPosition";
import "./version-history-badge.css";

export interface VersionSummary {
  workspaceId: string;
  fileId: string;
  versions: { id: string; versionNumber: number; producedAt: number; isCurrent: boolean; note?: string; isMilestone?: boolean }[];
}
interface Props {
  name: string;
  count: number;
  disabled: boolean;
  busy: boolean;
  loadSummary: () => Promise<VersionSummary>;
  onOpen: (anchor: HTMLButtonElement, versionId?: string) => void;
  onContextMenu: (event: MouseEvent<HTMLButtonElement>) => void;
}
const openedEvent = "file-version-summary-opened";

/** Returns the original sibling badge button, not a wrapper around the file button. */
export function VersionHistoryBadge({ name, count, disabled, busy, loadSummary, onOpen, onContextMenu }: Props) {
  const { t, i18n } = useTranslation();
  const id = useId();
  const badge = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const focusPanel = useRef(false);
  const loader = useRef(loadSummary);
  loader.current = loadSummary;
  const [open, setOpen] = useState(false);
  const [summary, setSummary] = useState<VersionSummary | null>(null);
  useVersionAnnotationUpdates(setSummary);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [position, setPosition] = useState<ReturnType<typeof versionSummaryPosition> | null>(null);
  const key = "fileSpace.versionSummary.";
  const clearTimers = useCallback(() => { clearTimeout(openTimer.current); clearTimeout(closeTimer.current); }, []);
  const close = useCallback(() => {
    clearTimers(); setOpen(false); setSummary(null); setError(false); focusPanel.current = false;
  }, [clearTimers]);
  const show = (keyboard = false) => {
    clearTimers();
    if (disabled || document.querySelector('[aria-modal="true"]')) return;
    focusPanel.current = keyboard;
    window.dispatchEvent(new CustomEvent(openedEvent, { detail: id }));
    setOpen(true);
  };
  const scheduleOpen = () => {
    clearTimers();
    if (!open) openTimer.current = setTimeout(() => show(), 350);
  };
  const scheduleClose = () => {
    clearTimers();
    closeTimer.current = setTimeout(() => {
      if (!panel.current?.contains(document.activeElement) && document.activeElement !== badge.current) close();
    }, 200);
  };
  const openHistory = (versionId?: string) => {
    close();
    if (badge.current) onOpen(badge.current, versionId);
  };

  useEffect(() => {
    const otherOpened = (event: Event) => { if ((event as CustomEvent<string>).detail !== id) close(); };
    window.addEventListener(openedEvent, otherOpened);
    return () => { clearTimers(); window.removeEventListener(openedEvent, otherOpened); };
  }, [clearTimers, close, id]);
  useEffect(() => { if (disabled) close(); }, [disabled, close]);
  useEffect(() => {
    if (!open) return;
    let active = true;
    setError(false); setSummary(null);
    void loader.current().then((loaded) => { if (active) setSummary(loaded); })
      .catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [open, attempt]);

  useLayoutEffect(() => {
    if (!open) return;
    const positionPanel = () => {
      if (!badge.current || !panel.current) return;
      const body = panel.current.querySelector<HTMLElement>(".file-version-summary-body");
      // Measure the natural content, not the currently constrained scroll box;
      // otherwise a short loading state can pin a long result to the wrong side.
      const naturalHeight = panel.current.getBoundingClientRect().height
        + (body ? body.scrollHeight - body.clientHeight : 0);
      const next = versionSummaryPosition(badge.current.getBoundingClientRect(),
        { width: window.innerWidth, height: window.innerHeight }, naturalHeight);
      setPosition((current) => JSON.stringify(current) === JSON.stringify(next) ? current : next);
    };
    positionPanel();
    const observer = new ResizeObserver(positionPanel);
    if (panel.current) observer.observe(panel.current);
    window.addEventListener("resize", positionPanel);
    if (focusPanel.current) { panel.current?.focus({ preventScroll: true }); focusPanel.current = false; }
    return () => { observer.disconnect(); window.removeEventListener("resize", positionPanel); };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (!panel.current?.contains(event.target as Node) && !badge.current?.contains(event.target as Node)) close();
    };
    const scroll = (event: Event) => { if (!panel.current?.contains(event.target as Node)) close(); };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault(); event.stopImmediatePropagation();
      const returnFocus = panel.current?.contains(document.activeElement);
      close(); if (returnFocus) badge.current?.focus({ preventScroll: true });
      clearTimers(); // Focus restoration must not reopen the just-dismissed summary.
    };
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("scroll", scroll, true);
    window.addEventListener("keydown", escape, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("scroll", scroll, true);
      window.removeEventListener("keydown", escape, true);
      window.removeEventListener("blur", close);
    };
  }, [clearTimers, close, open]);

  return <>
    <button ref={badge} className="file-space-file-version-count is-history-action" type="button"
      aria-label={`${t("fileSpace.fileMenu.versionHistory")} · ${name} · ${t("fileSpace.content.versionCount", { count })}`}
      aria-expanded={open} aria-controls={open ? id : undefined} aria-haspopup="dialog"
      aria-busy={busy} disabled={disabled} onPointerDown={(event) => event.stopPropagation()}
      onPointerEnter={(event) => { if (event.pointerType !== "touch") scheduleOpen(); }} onPointerLeave={scheduleClose}
      onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) scheduleOpen(); }}
      onBlur={(event) => { if (!panel.current?.contains(event.relatedTarget)) close(); }}
      onDoubleClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        if (["Enter", " ", "ArrowDown", "Tab"].includes(event.key)) event.stopPropagation();
        if (event.key === "ArrowDown") { event.preventDefault(); if (open) panel.current?.focus(); else show(true); }
        if (event.key === "Tab" && !event.shiftKey && open) { event.preventDefault(); panel.current?.querySelector<HTMLButtonElement>("button")?.focus(); }
      }}
      onContextMenu={(event) => { close(); onContextMenu(event); }}
      onClick={(event) => { event.stopPropagation(); if (event.detail > 1) return; openHistory(); }}>
      {busy ? <LoaderCircle size={11} className="is-spinning" aria-hidden="true" /> : <Clock3 size={11} aria-hidden="true" />}
      <span>{t("fileSpace.content.versionCount", { count })}</span>
    </button>
    {open ? createPortal(<div ref={panel} id={id} className="file-version-summary" role="dialog" aria-modal="false"
      aria-labelledby={`${id}-title`} tabIndex={-1} data-side={position?.side}
      style={{ left: position?.left, top: position?.top, width: position?.width ?? 320, maxHeight: position?.maxHeight ?? 340,
        visibility: position ? "visible" : "hidden", "--summary-arrow": `${position?.arrow ?? 24}px` } as React.CSSProperties}
      onPointerEnter={clearTimers} onPointerLeave={scheduleClose}
      onPointerDown={(event) => event.stopPropagation()} onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()} onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); }}
      onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget) && event.relatedTarget !== badge.current) close(); }}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Tab") {
          const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("button"));
          if ((event.shiftKey && (document.activeElement === buttons[0] || document.activeElement === panel.current))
            || (!event.shiftKey && document.activeElement === buttons.at(-1))) {
            event.preventDefault(); close(); badge.current?.focus({ preventScroll: true }); clearTimers();
          }
        }
      }}>
      <header><strong id={`${id}-title`}>{t(key + "title")}</strong><small>{t("fileSpace.content.versionCount", { count })}</small></header>
      <p className="file-version-summary-name" title={name}>{name}</p>
      <div className="file-version-summary-body" aria-busy={!summary && !error}>
        {error ? <div className="file-version-summary-state" role="status"><span>{t(key + "loadError")}</span>
          <button type="button" onClick={() => setAttempt((current) => current + 1)}>{t(key + "retry")}</button></div>
          : !summary ? <p className="file-version-summary-state" role="status">{t("fileSpace.timeline.loading")}</p>
          : summary.versions.length === 0 ? <p className="file-version-summary-state">{t(key + "empty")}</p>
          : <ol>{summary.versions.map((version) => <li key={version.id}>
            <button type="button" onClick={() => openHistory(version.id)}>
              <span className="file-version-summary-row-heading"><strong><VersionName number={version.versionNumber} /></strong>
                {version.isCurrent ? <small>{t("fileSpace.timeline.current")}</small> : null}
                {version.isMilestone ? <Star className="file-version-summary-star" size={14} aria-label={t(key + "milestone")} /> : null}
              </span>
              <time dateTime={new Date(version.producedAt).toISOString()}>{new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: "short", timeStyle: "short" }).format(version.producedAt)}</time>
              {version.note ? <span className="file-version-summary-note" title={version.note}>{version.note}</span> : null}
            </button>
          </li>)}</ol>}
      </div>
      <footer><button type="button" onClick={() => openHistory()}>{t(key + "viewAll")}</button></footer>
    </div>, document.body) : null}
  </>;
}
