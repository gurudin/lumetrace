import { invoke, isTauri } from "@tauri-apps/api/core";
import { AlertTriangle, History, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import "./text-preview-overlay.css";

interface TextPreviewRequest {
  fileId: string;
  name: string;
  hasVersionHistory: boolean;
  versionCount: number;
}

interface TaskFileVersionRecord {
  id: string;
  versionNumber: number;
  origin: "task" | "user_edit";
  taskTitle: string | null;
  roundNumber: number | null;
  cellName: string | null;
  producedAt: number;
  isCurrent: boolean;
}

interface TaskFileTimelineRecord {
  fileId: string;
  currentVersionId: string;
  versions: TaskFileVersionRecord[];
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function TextPreviewOverlay() {
  const { i18n } = useTranslation();
  const zh = i18n.resolvedLanguage?.startsWith("zh") ?? true;
  const locale = zh ? "zh-CN" : "en-US";
  const copy = zh ? {
    dialog: (name: string) => `预览 TXT：${name}`,
    close: "关闭 TXT 预览",
    loading: "正在读取 TXT",
    loadError: "无法读取这个 TXT 文件。",
    retry: "重新读取",
    empty: "这个 TXT 文件当前没有内容。",
    desktopOnly: "TXT 文件只能在 LumeTrace 客户端中读取。",
    versionHistory: "版本记录",
    versionCount: (count: number) => `共 ${count} 个版本`,
    current: "当前版本",
    historical: "历史版本 · 只读",
    userEdit: "用户修改",
    round: (count: number) => `第 ${count} 轮`,
    timelineError: "无法读取版本记录",
  } : {
    dialog: (name: string) => `Preview TXT: ${name}`,
    close: "Close TXT preview",
    loading: "Loading TXT",
    loadError: "Unable to read this TXT file.",
    retry: "Reload",
    empty: "This TXT file is empty.",
    desktopOnly: "TXT files can only be read in the LumeTrace desktop app.",
    versionHistory: "Version history",
    versionCount: (count: number) => `${count} version${count === 1 ? "" : "s"}`,
    current: "Current",
    historical: "Historical version · Read only",
    userEdit: "User edit",
    round: (count: number) => `Round ${count}`,
    timelineError: "Unable to load version history",
  };
  const [request, setRequest] = useState<TextPreviewRequest | null>(null);
  const [open, setOpen] = useState(false);
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<TaskFileTimelineRecord | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const loadSequenceRef = useRef(0);
  const presence = usePresence(open);
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const showTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);

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
    if (!target.hasVersionHistory || target.versionCount <= 0) return;
    setTimelineLoading(true);
    setTimelineError(null);
    try {
      const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: target.fileId });
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
    } catch (error) {
      setTimelineError(errorText(error));
    } finally {
      setTimelineLoading(false);
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
      setRequest(target);
      setContent("");
      setTimeline(null);
      setTimelineError(null);
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
    const close = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      closePreview();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [closePreview, open]);

  useEffect(() => {
    if (!presence.mounted) {
      loadSequenceRef.current += 1;
      setRequest(null);
      setTimeline(null);
    }
  }, [presence.mounted]);

  if (!presence.mounted || !request) return null;

  return (
    <div className="file-text-preview" data-state={presence.state} role="dialog" aria-modal="true" aria-label={copy.dialog(request.name)}>
      <header className="file-text-preview-toolbar">
        <div>
          <strong title={request.name}>{request.name}</strong>
          {selectedVersion && !selectedVersion.isCurrent ? <small>{copy.historical}</small> : null}
        </div>
        <button ref={closeButtonRef} type="button" onClick={closePreview} title={copy.close} aria-label={copy.close}><X size={18} /></button>
      </header>
      <div className={`file-text-preview-workspace${showTimeline ? " has-version-timeline" : ""}`}>
        {showTimeline ? (
          <aside className="file-text-version-timeline" aria-label={copy.versionHistory}>
            <header><div><History size={15} /><span>{copy.versionHistory}</span></div><small>{copy.versionCount(timeline?.versions.length ?? request.versionCount)}</small></header>
            {timelineLoading ? <div className="file-text-preview-state"><LoaderCircle className="is-spinning" size={17} /><span>{copy.loading}</span></div>
              : timelineError ? <div className="file-text-preview-state is-error"><AlertTriangle size={17} /><strong>{copy.timelineError}</strong><span>{timelineError}</span><button type="button" onClick={() => void loadTimeline(request)}>{copy.retry}</button></div>
                : timeline ? (
                  <ol className="file-text-version-list">
                    {timeline.versions.map((version) => (
                      <li key={version.id} className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}>
                        <button type="button" aria-pressed={selectedVersionId === version.id} disabled={loading} onClick={() => void selectVersion(version)}>
                          <time dateTime={new Date(version.producedAt).toISOString()}>{new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(version.producedAt)}</time>
                          <strong>v{version.versionNumber}{version.isCurrent ? <span>{copy.current}</span> : null}</strong>
                          <span>{version.cellName ?? (version.origin === "user_edit" ? copy.userEdit : version.taskTitle)}</span>
                          {version.roundNumber ? <small>{copy.round(version.roundNumber)}</small> : null}
                        </button>
                      </li>
                    ))}
                  </ol>
                ) : null}
          </aside>
        ) : null}
        <main className="file-text-preview-body">
          {loading ? <div className="file-text-preview-state"><LoaderCircle className="is-spinning" size={22} /><span>{copy.loading}</span></div>
            : loadError ? <div className="file-text-preview-state is-error"><AlertTriangle size={22} /><strong>{copy.loadError}</strong><span>{loadError}</span><button type="button" onClick={() => void (selectedVersion ? selectVersion(selectedVersion, true) : loadCurrent(request))}>{copy.retry}</button></div>
              : content ? <pre>{content}</pre> : <p>{copy.empty}</p>}
        </main>
      </div>
    </div>
  );
}
