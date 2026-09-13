import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { VersionAuthor } from "./VersionAuthor";
import { isTauri } from "@tauri-apps/api/core";
import { AlertTriangle, Eye, History, LoaderCircle, Pencil, Save, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { usePresence } from "../../shared/ui/usePresence";
import { VersionTimelineRegion, VersionTimelineToggle } from "./VersionTimelineToggle";
import { shouldShowVersionTimelineByDefault } from "./versionTimelineVisibility";
import { PreviewFileHeading } from "./PreviewFileHeading";
import { InitialVersionHint, VersionName } from "./InitialVersionHint";
import { claimFirstVersionChange, savedVersionNotification, viewVersionChangeEvent } from "./firstVersionChange";
import type { FileSpaceVersionNotification } from "./versionNotification";
import { PreviewDiffButton, PreviewVersionDiff } from "./PreviewVersionDiff";
import { VersionAnnotation, useVersionAnnotationUpdates } from "./VersionAnnotation";
import {
  fileOpenSearchContextFromEvent,
  type FileOpenSearchContext,
} from "./fileOpenSearchContext";
import { useSearchResultHighlight } from "./searchResultHighlight";
import "./markdown-preview-overlay.css";
import "./search-result-highlight.css";

interface MarkdownPreviewRequest {
  fileId: string;
  name: string;
  hasVersionHistory: boolean;
  versionCount: number;
  currentVersion: number | null;
}

interface TaskFileVersionRecord {
  authorName?: string | null;
  note?: string;
  isMilestone?: boolean;
  id: string;
  versionNumber: number;
  name: string;
  mimeType: string | null;
  sizeBytes: number;
  origin: "task" | "user_edit";
  taskId: string | null;
  taskTitle: string | null;
  roundNumber: number | null;
  cellId: string | null;
  cellName: string | null;
  producedAt: number;
  isCurrent: boolean;
}

interface TaskFileTimelineRecord {
  workspaceId?: string;
  fileId: string;
  logicalKey: string;
  currentVersionId: string;
  versions: TaskFileVersionRecord[];
}

type MarkdownMode = "preview" | "edit" | "diff";

interface MarkdownAstNode {
  type?: string;
  position?: {
    start?: { line?: number };
    end?: { line?: number };
  };
  properties?: Record<string, unknown>;
  children?: MarkdownAstNode[];
}

function rehypeSourceLines() {
  return (tree: MarkdownAstNode) => {
    const visit = (node: MarkdownAstNode) => {
      const start = node.position?.start?.line;
      const end = node.position?.end?.line;
      if (node.type === "element" && start) {
        node.properties = {
          ...node.properties,
          dataSourceStartLine: start,
          dataSourceEndLine: end ?? start,
        };
      }
      node.children?.forEach(visit);
    };
    visit(tree);
  };
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function isMarkdownName(name: string) {
  return /\.(md|markdown)$/i.test(name.trim());
}

const markdownVisualFixture = `# Clipboard X 市场调研报告

> 本报告用于验证 Lume Trace Markdown 阅读与编辑器在真实密度内容下的排版、滚动与交互。

## 核心结论

- 用户希望跨设备同步，但不愿意让敏感剪贴板内容默认上传云端。
- 搜索速度、历史容量和快捷键稳定性是高频痛点。
- 产品应先解决“快速找回”和“可控整理”，再扩展团队共享。

## 竞品对比

| 产品 | 优势 | 主要吐槽 | 差异化机会 |
| --- | --- | --- | --- |
| Clipboard Manager | 上手简单 | 长历史下搜索不稳定 | 本地语义检索 |
| Paste | 视觉体验完整 | 订阅成本较高 | 买断与本地优先 |
| Maccy | 轻量开源 | 缺少知识整理 | AI 自动标签 |

## 用户任务

- [x] 收集公开评论
- [x] 归类核心痛点
- [ ] 验证本地语义搜索
- [ ] 完成 MVP 访谈

## 数据结构示例

\`\`\`ts
interface ClipboardRecord {
  id: string;
  content: string;
  tags: string[];
  capturedAt: number;
}
\`\`\`

## 推广假设

第一阶段以开发者、设计师和高频办公用户为种子群体。内容推广不强调“又一个剪贴板工具”，而是展示从复制、遗忘到快速找回的完整工作流。

### 风险与验证

1. AI 整理是否真的节省时间，而不是增加确认成本。
2. 本地索引在大规模历史数据下是否仍然流畅。
3. 用户是否愿意为跨设备同步付费。

---

后续证据与访谈记录应继续追加到同一逻辑文件的版本历史中。`;

function visualFixtureEnabled(target: MarkdownPreviewRequest) {
  return import.meta.env.DEV
    && new URLSearchParams(window.location.search).has("fileSpacePreview")
    && target.fileId.startsWith("fixture-file-");
}

function markdownFixtureForVersion(versionNumber: number, currentVersion: number | null) {
  if (versionNumber === currentVersion) return markdownVisualFixture;
  return `# Clipboard X 市场调研报告 · v${versionNumber}\n\n> 这是用于验证版本时间线的只读历史内容。\n\n${markdownVisualFixture}`;
}

function createTimelineVisualFixture(target: MarkdownPreviewRequest): TaskFileTimelineRecord {
  const versionCount = Math.max(1, target.versionCount);
  const currentVersion = Math.min(versionCount, Math.max(1, target.currentVersion ?? versionCount));
  const now = Date.now();
  const versions = Array.from({ length: versionCount }, (_, index): TaskFileVersionRecord => {
    const versionNumber = versionCount - index;
    return {
      id: `fixture-version-${versionNumber}`,
      versionNumber,
      name: target.name,
      mimeType: "text/markdown",
      sizeBytes: 8_192 + versionNumber * 256,
      origin: "task",
      taskId: "fixture-task",
      taskTitle: "Clipboard X",
      roundNumber: versionNumber,
      cellId: versionNumber === 1 ? "fixture-general-cell" : "fixture-product-cell",
      cellName: versionNumber === 1 ? "通用 Cell" : "产品策略 Cell",
      producedAt: now - (versionCount - versionNumber) * 86_400_000,
      isCurrent: versionNumber === currentVersion,
    };
  });
  return {
    fileId: target.fileId,
    workspaceId: "visual-preview",
    logicalKey: "artifact-test",
    currentVersionId: `fixture-version-${currentVersion}`,
    versions,
  };
}

export function MarkdownPreviewOverlay() {
  const invoke = useWorkspaceInvoke();
  const { t, i18n } = useTranslation();
  const copy = {
    dialog: (name: string) => t("fileSpace.preview.markdown.dialog", { name }),
    preview: t("fileSpace.preview.markdown.preview"),
    edit: t("fileSpace.preview.markdown.edit"),
    save: t("fileSpace.preview.markdown.save"),
    saving: t("fileSpace.preview.markdown.saving"),
    saved: t("fileSpace.preview.markdown.saved"),
    unsaved: t("fileSpace.preview.markdown.unsaved"),
    close: t("fileSpace.preview.markdown.close"),
    loading: t("fileSpace.preview.markdown.loading"),
    loadError: t("fileSpace.preview.markdown.loadError"),
    saveError: t("fileSpace.preview.markdown.saveError"),
    retry: t("fileSpace.preview.markdown.retry"),
    editorLabel: t("fileSpace.preview.markdown.editorLabel"),
    empty: t("fileSpace.preview.markdown.empty"),
    discardTitle: t("fileSpace.preview.markdown.discardTitle"),
    discardBody: t("fileSpace.preview.markdown.discardBody"),
    keepEditing: t("fileSpace.preview.markdown.keepEditing"),
    discard: t("fileSpace.preview.markdown.discard"),
    desktopOnly: t("fileSpace.preview.markdown.desktopOnly"),
    versionHistory: t("fileSpace.preview.common.versionHistory"),
    showVersionHistory: t("fileSpace.preview.common.showVersionHistory"),
    hideVersionHistory: t("fileSpace.preview.common.hideVersionHistory"),
    versionCount: (count: number) => t("fileSpace.preview.common.versionCount", { count }),
    currentVersion: t("fileSpace.preview.common.current"),
    historicalVersion: t("fileSpace.preview.common.historical"),
    userEdit: t("fileSpace.preview.common.userEdit"),
    timelineLoadError: t("fileSpace.preview.common.timelineError"),
    versionLoadError: t("fileSpace.preview.markdown.versionLoadError"),
    retryTimeline: t("fileSpace.preview.common.retry"),
    versionLoading: t("fileSpace.preview.markdown.versionLoading"),
    round: (count: number) => t("fileSpace.preview.common.round", { count }),
  };
  const [request, setRequest] = useState<MarkdownPreviewRequest | null>(null);
  const [searchContext, setSearchContext] = useState<FileOpenSearchContext | null>(null);
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<MarkdownMode>("preview");
  const [content, setContent] = useState("");
  const [draft, setDraft] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [firstVersionChange, setFirstVersionChange] = useState<FileSpaceVersionNotification | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [timeline, setTimeline] = useState<TaskFileTimelineRecord | null>(null);
  useVersionAnnotationUpdates(setTimeline);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [timelineVisible, setTimelineVisible] = useState(false);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [versionLoading, setVersionLoading] = useState(false);
  const [versionError, setVersionError] = useState<string | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const documentRef = useRef<HTMLElement>(null);
  const discardButtonRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const contentLoadSequenceRef = useRef(0);
  const initialLoadSequenceRef = useRef(0);
  const timelineLoadSequenceRef = useRef(0);
  const presence = usePresence(open);
  const discardPresence = usePresence(confirmClose);
  const dirty = draft !== content;
  const selectedVersion = timeline?.versions.find((version) => version.id === selectedVersionId) ?? null;
  const historicalVersionSelected = Boolean(selectedVersion && !selectedVersion.isCurrent);
  const hasVersionTimeline = Boolean(request?.hasVersionHistory && request.versionCount > 0);
  const showVersionTimeline = hasVersionTimeline && timelineVisible;
  useSearchResultHighlight(
    documentRef,
    mode === "preview" && !historicalVersionSelected ? searchContext : null,
    draft,
  );
  const readDiffSnapshot = useCallback(async (fileId: string, versionId: string) => {
    if (request && visualFixtureEnabled(request)) {
      const version = timeline?.versions.find((candidate) => candidate.id === versionId);
      if (!version) throw new Error("Unknown fixture version");
      return Array.from(new TextEncoder().encode(markdownFixtureForVersion(version.versionNumber, request.currentVersion)));
    }
    return invoke<number[]>("read_task_file_version", { fileId, versionId });
  }, [request, timeline]);

  const loadMarkdown = useCallback(async (target: MarkdownPreviewRequest) => {
    const loadSequence = contentLoadSequenceRef.current + 1;
    contentLoadSequenceRef.current = loadSequence;
    setLoading(true);
    setLoadError(null);
    setVersionError(null);
    setSaveError(null);
    setSaved(false);
    try {
      const isVisualFixture = visualFixtureEnabled(target);
      if (!isTauri() && !isVisualFixture) throw new Error(copy.desktopOnly);
      const loaded = isVisualFixture
        ? markdownVisualFixture
        : await invoke<string>("read_file_space_markdown", { fileId: target.fileId });
      if (loadSequence !== contentLoadSequenceRef.current) return;
      setContent(loaded);
      setDraft(loaded);
    } catch (error) {
      if (loadSequence === contentLoadSequenceRef.current) setLoadError(errorText(error));
    } finally {
      if (loadSequence === contentLoadSequenceRef.current) setLoading(false);
    }
  }, [copy.desktopOnly]);

  const loadTaskTimeline = useCallback(async (target: MarkdownPreviewRequest) => {
    const sequence = ++timelineLoadSequenceRef.current;
    if (!target.hasVersionHistory || target.versionCount <= 0) {
      setTimeline(null);
      setSelectedVersionId(null);
      setTimelineError(null);
      return;
    }
    setTimelineLoading(true);
    setTimelineError(null);
    try {
      const loaded = visualFixtureEnabled(target)
        ? createTimelineVisualFixture(target)
        : await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: target.fileId });
      if (sequence !== timelineLoadSequenceRef.current) return;
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
      return loaded;
    } catch (error) {
      if (sequence !== timelineLoadSequenceRef.current) return;
      setTimeline(null);
      setSelectedVersionId(null);
      setTimelineError(errorText(error));
    } finally {
      if (sequence === timelineLoadSequenceRef.current) setTimelineLoading(false);
    }
  }, []);

  const selectTimelineVersion = useCallback(async (version: TaskFileVersionRecord, force = false) => {
    if (!request || dirty || saving || versionLoading || (!force && selectedVersionId === version.id)) return;
    const loadSequence = contentLoadSequenceRef.current + 1;
    contentLoadSequenceRef.current = loadSequence;
    setSelectedVersionId(version.id);
    setSearchContext(null);
    setVersionLoading(true);
    setVersionError(null);
    setLoadError(null);
    setMode((current) => current === "diff" ? "diff" : "preview");
    setSaved(false);
    try {
      let loaded: string;
      if (visualFixtureEnabled(request)) {
        loaded = markdownFixtureForVersion(version.versionNumber, request.currentVersion);
      } else if (version.isCurrent) {
        loaded = await invoke<string>("read_file_space_markdown", { fileId: request.fileId });
      } else {
        const bytes = await invoke<number[]>("read_task_file_version", {
          fileId: request.fileId,
          versionId: version.id,
        });
        loaded = new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
      }
      if (loadSequence !== contentLoadSequenceRef.current) return;
      setContent(loaded);
      setDraft(loaded);
    } catch (error) {
      if (loadSequence === contentLoadSequenceRef.current) setVersionError(errorText(error));
    } finally {
      if (loadSequence === contentLoadSequenceRef.current) setVersionLoading(false);
    }
  }, [dirty, request, saving, selectedVersionId, versionLoading]);

  const closeImmediately = useCallback((restoreFocus = true) => {
    initialLoadSequenceRef.current += 1;
    timelineLoadSequenceRef.current += 1;
    setConfirmClose(false);
    setOpen(false);
    if (restoreFocus) window.setTimeout(() => returnFocusRef.current?.focus(), 220);
  }, []);

  const requestClose = useCallback(() => {
    if (saving) return;
    if (dirty) {
      setConfirmClose(true);
      return;
    }
    closeImmediately();
  }, [closeImmediately, dirty, saving]);

  const saveMarkdown = useCallback(async () => {
    if (!request || saving || !dirty || historicalVersionSelected) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      const isVisualFixture = visualFixtureEnabled(request);
      const snapshot = isVisualFixture ? null : await invoke<unknown>("save_file_space_markdown", {
          fileId: request.fileId,
          content: draft,
        });
      setContent(draft);
      setMode("preview");
      setSaved(true);
      if (snapshot) {
        window.dispatchEvent(new CustomEvent("lumetrace:file-space-snapshot", { detail: snapshot }));
      }
      if (request.hasVersionHistory && !isVisualFixture) {
        const loaded = await loadTaskTimeline(request);
        if (loaded) {
          const notification = savedVersionNotification(request.fileId, request.name,
            Math.max(request.currentVersion ?? 0, ...(timeline?.versions.map((version) => version.versionNumber) ?? [])),
            loaded.versions);
          setRequest((current) => current?.fileId === request.fileId
            ? { ...current, versionCount: loaded.versions.length,
                currentVersion: loaded.versions.find((version) => version.isCurrent)?.versionNumber ?? current.currentVersion }
            : current);
          if (notification && claimFirstVersionChange(notification)) {
            setFirstVersionChange(notification);
            setTimelineVisible(true);
          }
        }
      }
    } catch (error) {
      setSaveError(errorText(error));
    } finally {
      setSaving(false);
    }
  }, [dirty, draft, historicalVersionSelected, loadTaskTimeline, request, saving, timeline]);

  useEffect(() => {
    const markdownCardFromTarget = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return null;
      const button = target.closest<HTMLButtonElement>(".file-space-file-card > button:first-child");
      const card = button?.closest<HTMLElement>(".file-space-file-card");
      const fileId = card?.dataset.fileId;
      const name = button?.title ?? "";
      if (!button || !fileId || !isMarkdownName(name)) return null;
      const parsedVersionCount = Number(card.dataset.versionCount ?? "0");
      const parsedCurrentVersion = Number(card.dataset.currentVersion ?? "0");
      return {
        button,
        request: {
          fileId,
          name,
          hasVersionHistory: Number(card.dataset.versionCount ?? 0) > 0,
          versionCount: Number.isFinite(parsedVersionCount) ? Math.max(0, parsedVersionCount) : 0,
          currentVersion: Number.isFinite(parsedCurrentVersion) && parsedCurrentVersion > 0
            ? parsedCurrentVersion
            : null,
        },
      };
    };

    const handleDoubleClick = (event: MouseEvent) => {
      const card = markdownCardFromTarget(event.target);
      if (!card || event.button !== 0) return;
      const interactionStartedAt = performance.now();
      const initialLoadSequence = initialLoadSequenceRef.current + 1;
      initialLoadSequenceRef.current = initialLoadSequence;
      timelineLoadSequenceRef.current += 1;
      event.preventDefault();
      event.stopImmediatePropagation();
      returnFocusRef.current = card.button;
      setRequest(card.request);
      setSearchContext(fileOpenSearchContextFromEvent(event, card.request.fileId));
      setMode("preview");
      setContent("");
      setDraft("");
      setLoading(true);
      setLoadError(null);
      setSaveError(null);
      setSaved(false);
      setConfirmClose(false);
      setFirstVersionChange(null);
      setTimeline(null);
      setTimelineLoading(false);
      setTimelineError(null);
      const timelineInitiallyVisible = shouldShowVersionTimelineByDefault();
      setTimelineVisible(timelineInitiallyVisible);
      setSelectedVersionId(null);
      setVersionError(null);
      setVersionLoading(false);
      setOpen(true);
      window.requestAnimationFrame(() => {
        window.setTimeout(() => {
          if (initialLoadSequence !== initialLoadSequenceRef.current) return;
          if (import.meta.env.DEV) {
            console.info(
              `[Lume Trace preview] shell visible in ${(performance.now() - interactionStartedAt).toFixed(1)} ms`,
              card.request.name,
            );
          }
          void loadMarkdown(card.request);
          if (timelineInitiallyVisible) void loadTaskTimeline(card.request);
        }, 0);
      });
    };

    document.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      document.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, [loadMarkdown, loadTaskTimeline]);

  useEffect(() => {
    if (!open || !presence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [open, presence.mounted]);

  useEffect(() => {
    if (!open) return undefined;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape") {
        event.preventDefault();
        if (confirmClose) setConfirmClose(false);
        else requestClose();
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveMarkdown();
        return;
      }
      if (event.key === "Tab") {
        const owner = confirmClose
          ? dialogRef.current?.querySelector<HTMLElement>(".file-markdown-discard-dialog")
          : dialogRef.current;
        const focusable = Array.from(owner?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), textarea:not(:disabled), a[href]'
        ) ?? []).filter((element) => element.offsetParent !== null);
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
  }, [confirmClose, open, requestClose, saveMarkdown, discardPresence.mounted]);

  useEffect(() => {
    if (mode === "edit" && !loading && !loadError) {
      window.requestAnimationFrame(() => editorRef.current?.focus());
    }
  }, [loadError, loading, mode]);

  useEffect(() => {
    if (confirmClose && discardPresence.mounted) {
      window.requestAnimationFrame(() => discardButtonRef.current?.focus());
    }
  }, [confirmClose, discardPresence.mounted]);

  useEffect(() => {
    if (!presence.mounted) {
      contentLoadSequenceRef.current += 1;
      initialLoadSequenceRef.current += 1;
      setRequest(null);
      setSearchContext(null);
      setConfirmClose(false);
      setTimeline(null);
      setTimelineError(null);
      setTimelineVisible(false);
      setSelectedVersionId(null);
      setVersionError(null);
    }
  }, [presence.mounted]);

  if (!presence.mounted || !request) return null;

  return (
    <div
      ref={dialogRef}
      className="file-markdown-preview"
      data-state={presence.state}
      role="dialog"
      aria-modal="true"
      aria-label={copy.dialog(request.name)}
    >
      <header className="file-markdown-preview-toolbar">
        <PreviewFileHeading name={request.name} versionCount={timeline?.versions.length ?? request.versionCount}>
          {!loading && !loadError && historicalVersionSelected ? <small>{copy.historicalVersion}</small> : null}
          {!loading && !loadError && dirty ? <small>{copy.unsaved}</small> : null}
          {!loading && !loadError && !dirty && saved ? <small>{copy.saved}</small> : null}
        </PreviewFileHeading>
        <div className="file-markdown-preview-actions">
          <div className="file-markdown-preview-mode" aria-label={copy.dialog(request.name)}>
            <button
              className={mode === "preview" ? "is-active" : ""}
              type="button"
              aria-pressed={mode === "preview"}
              disabled={loading || versionLoading || Boolean(loadError) || Boolean(versionError)}
              onClick={() => setMode("preview")}
            >
              <Eye size={15} />{copy.preview}
            </button>
            <PreviewDiffButton
              active={mode === "diff"}
              versionCount={timeline?.versions.length ?? request.versionCount}
              disabled={dirty || saving}
              disabledReason={dirty ? copy.unsaved : undefined}
              onClick={() => {
                setMode((current) => current === "diff" ? "preview" : "diff");
                if (!timeline && !timelineLoading) void loadTaskTimeline(request);
              }}
            />
            <button
              className={mode === "edit" ? "is-active" : ""}
              type="button"
              aria-pressed={mode === "edit"}
              disabled={loading || versionLoading || historicalVersionSelected || Boolean(loadError) || Boolean(versionError)}
              onClick={() => setMode("edit")}
            >
              <Pencil size={14} />{copy.edit}
            </button>
          </div>
          <button
            className="file-markdown-preview-save"
            type="button"
            disabled={loading || versionLoading || historicalVersionSelected || Boolean(loadError) || Boolean(versionError) || saving || !dirty}
            onClick={() => void saveMarkdown()}
          >
            {saving ? <LoaderCircle className="is-spinning" size={15} /> : <Save size={15} />}
            {saving ? copy.saving : copy.save}
          </button>
          <button
            ref={closeButtonRef}
            className="file-markdown-preview-close"
            type="button"
            disabled={saving}
            onClick={requestClose}
            aria-label={copy.close}
            title={copy.close}
          >
            <X size={18} />
          </button>
        </div>
      </header>

      {firstVersionChange ? (
        <div className="file-markdown-first-change" role="status" aria-live="polite">
          <History size={16} aria-hidden="true" />
          <div>
            <strong>{t("fileSpace.versionNotification.firstTitle")}</strong>
            <span>{t("fileSpace.versionNotification.firstDescription", { name: firstVersionChange.fileName })}</span>
          </div>
          <button type="button" disabled={dirty || saving} onClick={() => {
            closeImmediately(false);
            window.dispatchEvent(new CustomEvent(viewVersionChangeEvent, { detail: firstVersionChange }));
          }}>{t("fileSpace.versionNotification.viewChanges")}</button>
          <button type="button" className="file-markdown-first-change-dismiss" onClick={() => setFirstVersionChange(null)} aria-label={t("fileSpace.feedback.dismiss")}><X size={15} /></button>
        </div>
      ) : null}

      <div className={`file-markdown-preview-workspace file-preview-version-workspace${showVersionTimeline ? " has-version-timeline" : ""}`}>
        {hasVersionTimeline ? (
          <VersionTimelineRegion visible={showVersionTimeline}>
            <aside id="file-markdown-version-timeline" className="file-markdown-version-timeline" aria-label={copy.versionHistory}>
            <header>
              <div><History size={15} /><span>{copy.versionHistory}</span></div>
              <small>{copy.versionCount(timeline?.versions.length ?? request.versionCount)}</small>
            </header>

            {timelineLoading ? (
              <div className="file-markdown-timeline-state" role="status">
                <LoaderCircle className="is-spinning" size={17} />
                <span>{copy.loading}</span>
              </div>
            ) : timelineError ? (
              <div className="file-markdown-timeline-state is-error" role="alert">
                <AlertTriangle size={17} />
                <strong>{copy.timelineLoadError}</strong>
                <span>{timelineError}</span>
                <button type="button" onClick={() => void loadTaskTimeline(request)}>{copy.retryTimeline}</button>
              </div>
            ) : timeline ? (
              <ol className="file-markdown-version-list">
                {timeline.versions.map((version) => (
                  <li
                    key={version.id}
                    className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}
                  >
                    <button
                      type="button"
                      aria-pressed={selectedVersionId === version.id}
                      disabled={dirty || saving || versionLoading}
                      onClick={() => void selectTimelineVersion(version)}
                    >
                      <time dateTime={new Date(version.producedAt).toISOString()}>
                        {new Intl.DateTimeFormat(
                          i18n.resolvedLanguage ?? "en-US",
                          { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" },
                        ).format(version.producedAt)}
                      </time>
                      <strong>
                        <VersionName number={version.versionNumber} />
                        {version.isCurrent ? <span>{copy.currentVersion}</span> : null}
                      </strong>
                      <span>{version.origin === "user_edit" ? <VersionAuthor name={version.authorName} /> : (version.cellName ?? version.taskTitle)}</span>
                      {version.roundNumber ? <small>{copy.round(version.roundNumber)}</small> : null}
                    </button>
                    <VersionAnnotation workspaceId={timeline.workspaceId} fileId={request.fileId} version={version} disabled={dirty || saving} />
                  </li>
                ))}
              </ol>
            ) : null}
            {!timelineLoading && !timelineError ? <InitialVersionHint versions={timeline?.versions ?? null} /> : null}
            </aside>
          </VersionTimelineRegion>
        ) : null}

        {hasVersionTimeline ? (
          <VersionTimelineToggle
            controlsId="file-markdown-version-timeline"
            visible={showVersionTimeline}
            showLabel={copy.showVersionHistory}
            hideLabel={copy.hideVersionHistory}
            onToggle={() => {
              const nextVisible = !timelineVisible;
              setTimelineVisible(nextVisible);
              if (nextVisible && !timeline && !timelineLoading) void loadTaskTimeline(request);
            }}
          />
        ) : null}

        <main className={`file-markdown-preview-body is-${mode}`}>
          {mode === "diff" ? (open ? (
            <PreviewVersionDiff
              fileId={request.fileId}
              selectedVersionId={selectedVersionId}
              versions={timeline?.versions}
              loading={timelineLoading}
              error={timelineError}
              onRetry={() => void loadTaskTimeline(request)}
              readSnapshot={readDiffSnapshot}
            />
          ) : null) : loading || versionLoading ? (
            <div className="file-markdown-preview-state" role="status">
              <LoaderCircle className="is-spinning" size={22} />
              <span>{versionLoading ? copy.versionLoading : copy.loading}</span>
            </div>
          ) : loadError || versionError ? (
            <div className="file-markdown-preview-state is-error" role="alert">
              <AlertTriangle size={22} />
              <strong>{versionError ? copy.versionLoadError : copy.loadError}</strong>
              <span>{versionError ?? loadError}</span>
              <button
                type="button"
                onClick={() => {
                  if (versionError && selectedVersion) void selectTimelineVersion(selectedVersion, true);
                  else void loadMarkdown(request);
                }}
              >
                {copy.retry}
              </button>
            </div>
          ) : mode === "edit" ? (
            <textarea
              ref={editorRef}
              value={draft}
              onChange={(event) => {
                setDraft(event.target.value);
                setSaved(false);
                setSaveError(null);
              }}
              aria-label={copy.editorLabel}
            />
          ) : (
            <article ref={documentRef} className="file-markdown-document" data-native-context-menu="true">
              {draft ? (
                <ReactMarkdown
                  remarkPlugins={[remarkGfm]}
                  rehypePlugins={[rehypeSourceLines]}
                  components={{
                    a: ({ children, ...props }) => (
                      <a {...props} target="_blank" rel="noreferrer noopener">{children}</a>
                    ),
                  }}
                >
                  {draft}
                </ReactMarkdown>
              ) : <p className="file-markdown-empty">{copy.empty}</p>}
            </article>
          )}
        </main>
      </div>

      {saveError ? (
        <div className="file-markdown-preview-error" role="alert">
          <strong>{copy.saveError}</strong><span>{saveError}</span>
        </div>
      ) : null}

      {discardPresence.mounted ? (
        <div className="file-markdown-discard-backdrop" data-state={discardPresence.state}>
          <div className="file-markdown-discard-dialog" role="alertdialog" aria-modal="true" aria-labelledby="markdown-discard-title">
            <AlertTriangle size={20} />
            <div>
              <strong id="markdown-discard-title">{copy.discardTitle}</strong>
              <p>{copy.discardBody}</p>
            </div>
            <div className="file-markdown-discard-actions">
              <button type="button" onClick={() => setConfirmClose(false)}>{copy.keepEditing}</button>
              <button ref={discardButtonRef} className="is-danger" type="button" onClick={() => closeImmediately()}>{copy.discard}</button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
