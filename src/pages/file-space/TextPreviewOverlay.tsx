import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { VersionAuthor } from "./VersionAuthor";
import { isTauri } from "@tauri-apps/api/core";
import { AlertTriangle, History, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { VersionTimelineRegion, VersionTimelineToggle } from "./VersionTimelineToggle";
import { shouldShowVersionTimelineByDefault } from "./versionTimelineVisibility";
import { PreviewFileHeading } from "./PreviewFileHeading";
import { InitialVersionHint, VersionName } from "./InitialVersionHint";
import { PreviewDiffButton, PreviewVersionDiff } from "./PreviewVersionDiff";
import { VersionAnnotation, useVersionAnnotationUpdates } from "./VersionAnnotation";
import "./text-preview-overlay.css";

interface TextPreviewRequest {
  fileId: string;
  name: string;
  hasVersionHistory: boolean;
  versionCount: number;
}

interface TaskFileVersionRecord {
  authorName?: string | null;
  note?: string;
  isMilestone?: boolean;
  id: string;
  versionNumber: number;
  sizeBytes: number;
  origin: "task" | "user_edit";
  taskTitle: string | null;
  roundNumber: number | null;
  cellName: string | null;
  producedAt: number;
  isCurrent: boolean;
}

interface TaskFileTimelineRecord {
  workspaceId?: string;
  fileId: string;
  currentVersionId: string;
  versions: TaskFileVersionRecord[];
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function TextPreviewOverlay() {
  const invoke = useWorkspaceInvoke();
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? "en-US";
  const copy = {
    dialog: (name: string) => t("fileSpace.preview.text.dialog", { name }),
    close: t("fileSpace.preview.text.close"),
    loading: t("fileSpace.preview.text.loading"),
    loadError: t("fileSpace.preview.text.loadError"),
    retry: t("fileSpace.preview.text.retry"),
    empty: t("fileSpace.preview.text.empty"),
    desktopOnly: t("fileSpace.preview.text.desktopOnly"),
    versionHistory: t("fileSpace.preview.common.versionHistory"),
    showVersionHistory: t("fileSpace.preview.common.showVersionHistory"),
    hideVersionHistory: t("fileSpace.preview.common.hideVersionHistory"),
    versionCount: (count: number) => t("fileSpace.preview.common.versionCount", { count }),
    current: t("fileSpace.preview.common.current"),
    historical: t("fileSpace.preview.common.historical"),
    userEdit: t("fileSpace.preview.common.userEdit"),
    round: (count: number) => t("fileSpace.preview.common.round", { count }),
    timelineError: t("fileSpace.preview.common.timelineError"),
  };
  const [request, setRequest] = useState<TextPreviewRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [diffVisible, setDiffVisible] = useState(false);
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<TaskFileTimelineRecord | null>(null);
  useVersionAnnotationUpdates(setTimeline);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [timelineVisible, setTimelineVisible] = useState(false);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const loadSequenceRef = useRef(0);
  const timelineLoadSequenceRef = useRef(0);
  const presence = usePresence(open);
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const hasTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);
  const showTimeline = hasTimeline && timelineVisible;

  const loadCurrent = useCallback(async (target: TextPreviewRequest) => {
    const sequence = loadSequenceRef.current + 1;
    loadSequenceRef.current = sequence;
    setLoading(true);
    setLoadError(null);
    try {
      if (!isTauri()) throw new Error(copy.desktopOnly);
      const loaded = await invoke<string>("read_file_space_text", { fileId: target.fileId });
      if (sequence === loadSequenceRef.current) setContent(loaded);
    } catch (error) {
      if (sequence === loadSequenceRef.current) setLoadError(errorText(error));
    } finally {
      if (sequence === loadSequenceRef.current) setLoading(false);
    }
  }, [copy.desktopOnly]);

  const loadTimeline = useCallback(async (target: TextPreviewRequest) => {
    const sequence = ++timelineLoadSequenceRef.current;
    if (!target.hasVersionHistory || target.versionCount <= 0) return;
    setTimelineLoading(true);
    setTimelineError(null);
    try {
      const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: target.fileId });
      if (sequence !== timelineLoadSequenceRef.current) return;
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
    } catch (error) {
      if (sequence !== timelineLoadSequenceRef.current) return;
      setTimelineError(errorText(error));
    } finally {
      if (sequence === timelineLoadSequenceRef.current) setTimelineLoading(false);
    }
  }, []);

  const selectVersion = useCallback(async (version: TaskFileVersionRecord, force = false) => {
    if (!request || loading || (!force && selectedVersionId === version.id)) return;
    const sequence = loadSequenceRef.current + 1;
    loadSequenceRef.current = sequence;
    setSelectedVersionId(version.id);
    setLoading(true);
    setLoadError(null);
    try {
      const loaded = version.isCurrent
        ? await invoke<string>("read_file_space_text", { fileId: request.fileId })
        : new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(
            await invoke<number[]>("read_task_file_version", { fileId: request.fileId, versionId: version.id }),
          ));
      if (sequence === loadSequenceRef.current) setContent(loaded);
    } catch (error) {
      if (sequence === loadSequenceRef.current) setLoadError(errorText(error));
    } finally {
      if (sequence === loadSequenceRef.current) setLoading(false);
    }
  }, [loading, request, selectedVersionId]);

  const closePreview = useCallback(() => {
    timelineLoadSequenceRef.current += 1;
    setOpen(false);
    window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, []);

  useEffect(() => {
    const handleDoubleClick = (event: MouseEvent) => {
      if (!(event.target instanceof Element) || event.button !== 0) return;
      const button = event.target.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
      const card = button?.closest<HTMLElement>(".file-space-file-card");
      const fileId = card?.dataset.fileId;
      const name = button?.title ?? "";
      if (!button || !fileId || !/\.txt$/i.test(name.trim())) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const versionCount = Number(card.dataset.versionCount ?? "0");
      const target: TextPreviewRequest = {
        fileId,
        name,
        hasVersionHistory: Number(card.dataset.versionCount ?? 0) > 0,
        versionCount: Number.isFinite(versionCount) ? Math.max(0, versionCount) : 0,
      };
      returnFocusRef.current = button;
      timelineLoadSequenceRef.current += 1;
      setDiffVisible(false);
      setRequest(target);
      setContent("");
      setTimeline(null);
      setTimelineError(null);
      setTimelineVisible(shouldShowVersionTimelineByDefault());
      setSelectedVersionId(null);
      setOpen(true);
      void loadCurrent(target);
      void loadTimeline(target);
    };
    document.addEventListener("dblclick", handleDoubleClick, true);
    return () => document.removeEventListener("dblclick", handleDoubleClick, true);
  }, [loadCurrent, loadTimeline]);

  useEffect(() => {
    if (!open) return undefined;
    window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape") {
        event.preventDefault();
        closePreview();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])',
      ) ?? []).filter((element) => element.offsetParent !== null);
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!dialogRef.current?.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [closePreview, open]);

  useEffect(() => {
    if (!presence.mounted) {
      loadSequenceRef.current += 1;
      setRequest(null);
      setTimeline(null);
      setTimelineVisible(false);
    }
  }, [presence.mounted]);

  if (!presence.mounted || !request) return null;

  return (
    <div ref={dialogRef} className="file-text-preview" data-state={presence.state} role="dialog" aria-modal="true" aria-label={copy.dialog(request.name)}>
      <header className="file-text-preview-toolbar">
        <PreviewFileHeading name={request.name} versionCount={timeline?.versions.length ?? request.versionCount}>
          {selectedVersion && !selectedVersion.isCurrent ? <small>{copy.historical}</small> : null}
        </PreviewFileHeading>
        <div className="file-text-preview-actions">
          <PreviewDiffButton
            active={diffVisible}
            versionCount={timeline?.versions.length ?? request.versionCount}
            onClick={() => setDiffVisible((visible) => !visible)}
          />
          <button ref={closeButtonRef} type="button" onClick={closePreview} title={copy.close} aria-label={copy.close}><X size={18} /></button>
        </div>
      </header>
      <div className={`file-text-preview-workspace file-preview-version-workspace${showTimeline ? " has-version-timeline" : ""}`}>
        {hasTimeline ? (
          <VersionTimelineRegion visible={showTimeline}>
            <aside id="file-text-version-timeline" className="file-text-version-timeline" aria-label={copy.versionHistory}>
            <header><div><History size={15} /><span>{copy.versionHistory}</span></div><small>{copy.versionCount(timeline?.versions.length ?? request.versionCount)}</small></header>
            {timelineLoading ? <div className="file-text-preview-state"><LoaderCircle className="is-spinning" size={17} /><span>{copy.loading}</span></div>
              : timelineError ? <div className="file-text-preview-state is-error"><AlertTriangle size={17} /><strong>{copy.timelineError}</strong><span>{timelineError}</span><button type="button" onClick={() => void loadTimeline(request)}>{copy.retry}</button></div>
                : timeline ? (
                  <ol className="file-text-version-list">
                    {timeline.versions.map((version) => (
                      <li key={version.id} className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}>
                        <button type="button" aria-pressed={selectedVersionId === version.id} disabled={loading} onClick={() => void selectVersion(version)}>
                          <time dateTime={new Date(version.producedAt).toISOString()}>{new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(version.producedAt)}</time>
                          <strong><VersionName number={version.versionNumber} />{version.isCurrent ? <span>{copy.current}</span> : null}</strong>
                          <span>{version.origin === "user_edit" ? <VersionAuthor name={version.authorName} /> : (version.cellName ?? version.taskTitle)}</span>
                          {version.roundNumber ? <small>{copy.round(version.roundNumber)}</small> : null}
                        </button>
                        <VersionAnnotation workspaceId={timeline.workspaceId} fileId={request.fileId} version={version} />
                      </li>
                    ))}
                  </ol>
                ) : null}
            {!timelineLoading && !timelineError ? <InitialVersionHint versions={timeline?.versions ?? null} /> : null}
            </aside>
          </VersionTimelineRegion>
        ) : null}
        {hasTimeline ? (
          <VersionTimelineToggle
            controlsId="file-text-version-timeline"
            visible={showTimeline}
            showLabel={copy.showVersionHistory}
            hideLabel={copy.hideVersionHistory}
            onToggle={() => setTimelineVisible((visible) => !visible)}
          />
        ) : null}
        <main className={`file-text-preview-body${diffVisible ? " is-diff" : ""}`}>
          {diffVisible ? (open ? <PreviewVersionDiff
            fileId={request.fileId}
            selectedVersionId={selectedVersionId}
            versions={timeline?.versions}
            loading={timelineLoading}
            error={timelineError}
            onRetry={() => void loadTimeline(request)}
          /> : null) : loading ? <div className="file-text-preview-state"><LoaderCircle className="is-spinning" size={22} /><span>{copy.loading}</span></div>
            : loadError ? <div className="file-text-preview-state is-error"><AlertTriangle size={22} /><strong>{copy.loadError}</strong><span>{loadError}</span><button type="button" onClick={() => void (selectedVersion ? selectVersion(selectedVersion, true) : loadCurrent(request))}>{copy.retry}</button></div>
              : content ? <pre data-native-context-menu="true">{content}</pre> : <p>{copy.empty}</p>}
        </main>
      </div>
    </div>
  );
}
