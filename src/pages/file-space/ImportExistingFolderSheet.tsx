import { FileClock, FolderOpen, FolderTree, LoaderCircle, Search, ShieldCheck } from "lucide-react";
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";

interface ImportExistingFolderSheetProps {
  path: string | null;
  busy: boolean;
  progress: {
    processed: number;
    total: number;
  } | null;
  error: string | null;
  returnFocusTarget: HTMLElement | null;
  onCancel: () => void;
  onChooseAgain: () => void;
  onConfirm: () => void;
}

function folderName(path: string) {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}

export function ImportExistingFolderSheet({
  path,
  busy,
  progress,
  error,
  returnFocusTarget,
  onCancel,
  onChooseAgain,
  onConfirm,
}: ImportExistingFolderSheetProps) {
  const { t } = useTranslation();
  const presence = usePresence(Boolean(path));
  const sheetRef = useRef<HTMLElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const renderedPathRef = useRef<string | null>(path);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const wasOpenRef = useRef(false);
  const restoreFocusPendingRef = useRef(false);
  if (path) renderedPathRef.current = path;
  const renderedPath = path ?? renderedPathRef.current;

  useEffect(() => {
    if (path && !wasOpenRef.current) {
      returnFocusRef.current = returnFocusTarget?.isConnected
        ? returnFocusTarget
        : document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      wasOpenRef.current = true;
      restoreFocusPendingRef.current = false;
    } else if (!path && wasOpenRef.current) {
      wasOpenRef.current = false;
      restoreFocusPendingRef.current = true;
    }
  }, [path, returnFocusTarget]);

  useEffect(() => {
    if (presence.mounted || path || !restoreFocusPendingRef.current) return;
    restoreFocusPendingRef.current = false;
    const target = returnFocusRef.current;
    returnFocusRef.current = null;
    if (target?.isConnected) window.requestAnimationFrame(() => target.focus());
  }, [path, presence.mounted]);

  useEffect(() => {
    if (!presence.mounted) renderedPathRef.current = null;
  }, [presence.mounted]);

  useEffect(() => {
    if (presence.state !== "open") return undefined;
    const animationFrame = window.requestAnimationFrame(() => {
      if (busy) sheetRef.current?.focus();
      else confirmRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(animationFrame);
  }, [busy, error, presence.state]);

  useEffect(() => {
    if (!path) return undefined;
    const keepFocusInSheet = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!busy) onCancel();
        return;
      }
      if (event.key !== "Tab") return;
      const buttons = Array.from(sheetRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])
        .filter((button) => button.offsetParent !== null);
      event.preventDefault();
      event.stopImmediatePropagation();
      if (buttons.length === 0) {
        sheetRef.current?.focus();
        return;
      }
      const first = buttons[0];
      const last = buttons[buttons.length - 1];
      const active = document.activeElement;
      const activeIndex = buttons.indexOf(active as HTMLButtonElement);
      if (!sheetRef.current?.contains(active) || activeIndex < 0) {
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey) {
        (active === first ? last : buttons[activeIndex - 1]).focus();
      } else {
        (active === last ? first : buttons[activeIndex + 1]).focus();
      }
    };
    window.addEventListener("keydown", keepFocusInSheet, true);
    return () => window.removeEventListener("keydown", keepFocusInSheet, true);
  }, [busy, onCancel, path]);

  if (!presence.mounted || !renderedPath) return null;

  return createPortal(
    <div
      className={`file-space-sheet-backdrop${presence.state === "open" ? " is-open" : ""}`}
    >
      <section
        ref={sheetRef}
        className={`file-space-import-sheet${presence.state === "open" ? " is-open" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-busy={busy}
        tabIndex={-1}
        aria-labelledby="file-space-import-sheet-title"
      >
        <header>
          <span className="file-space-import-sheet-icon"><FolderOpen size={27} /></span>
          <div>
            <h2 id="file-space-import-sheet-title">{t("fileSpace.importSheet.title")}</h2>
            <p>{t("fileSpace.importSheet.description")}</p>
          </div>
        </header>

        <div className="file-space-import-sheet-path">
          <FolderOpen size={17} />
          <span><strong>{folderName(renderedPath)}</strong><small title={renderedPath}>{renderedPath}</small></span>
          <button type="button" disabled={busy} onClick={onChooseAgain}>{t("fileSpace.importSheet.chooseAgain")}</button>
        </div>

        <section className="file-space-import-sheet-summary">
          <h3>{t("fileSpace.importSheet.willDo")}</h3>
          <ul>
            <li><FolderTree size={17} /><span><strong>{t("fileSpace.importSheet.hierarchyTitle")}</strong><small>{t("fileSpace.importSheet.hierarchyDescription")}</small></span></li>
            <li><FileClock size={17} /><span><strong>{t("fileSpace.importSheet.versionTitle")}</strong><small>{t("fileSpace.importSheet.versionDescription")}</small></span></li>
            <li><Search size={17} /><span><strong>{t("fileSpace.importSheet.searchTitle")}</strong><small>{t("fileSpace.importSheet.searchDescription")}</small></span></li>
          </ul>
        </section>

        {error ? <p className="file-space-import-sheet-error" role="alert">{error}</p> : null}

        {busy ? (
          <div className="file-space-import-sheet-progress" role="status" aria-live="polite">
            <div>
              <span>
                {progress?.total
                  ? t("fileSpace.importFeedback.importing")
                  : t("fileSpace.importFeedback.preparing")}
              </span>
              {progress?.total ? <strong>{progress.processed} / {progress.total}</strong> : null}
            </div>
            <progress
              max={progress?.total || undefined}
              value={progress?.total ? progress.processed : undefined}
              aria-label={progress?.total
                ? `${progress.processed} / ${progress.total}`
                : t("fileSpace.importFeedback.preparing")}
            />
          </div>
        ) : null}

        <footer>
          <span><ShieldCheck size={14} />{t("fileSpace.importSheet.privacy")}</span>
          <div>
            <button type="button" disabled={busy} onClick={onCancel}>{t("fileSpace.importSheet.cancel")}</button>
            <button ref={confirmRef} className="is-primary" type="button" disabled={busy} onClick={onConfirm}>
              {busy ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : null}
              <span aria-live="polite">
                {busy ? t("fileSpace.importSheet.importing") : t("fileSpace.importSheet.confirm")}
              </span>
            </button>
          </div>
        </footer>
      </section>
    </div>,
    document.body,
  );
}
