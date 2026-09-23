import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { isTauri } from "@tauri-apps/api/core";
import {
  ChevronDown,
  ChevronUp,
  ExternalLink,
  File,
  FileImage,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  LoaderCircle,
  Search,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import { globalSearchShortcutLabel, isGlobalSearchShortcut } from "./globalSearchShortcut";
import "./file-space-search-panel.css";
import { useApplicationExtension } from "../../shared/extensions/ApplicationExtension";
import { splitSearchText } from "./searchTextHighlight";
import type { FileOpenSearchContext } from "./fileOpenSearchContext";
import { SearchVisualPreview } from "./SearchVisualPreview";
import { visualPreviewKind, type SearchVisualRect } from "./searchVisualGeometry";

export type FileSpaceSearchScope = "name" | "content" | "tag";

export interface FileSpaceSearchFile {
  id: string;
  folderId: string | null;
  name: string;
  relativePath: string;
  mimeType: string | null;
  sizeBytes: number;
  sourceKind: string;
  manualOrder: number;
  currentVersion: number | null;
  versionCount: number;
  tags: string[];
  createdAt: number;
  updatedAt: number;
}

interface FileSpaceSearchFilePage {
  files: FileSpaceSearchFile[];
}

export interface FileSpaceSearchFolder {
  id: string;
  relativePath: string;
}

export interface FileSpaceSearchMatch {
  fileId: string;
  lexicalMatch: boolean;
  semanticSimilarity: number | null;
  contentMatch?: boolean;
  snippet?: string | null;
}

interface FileSpaceSearchPreviewSection {
  text: string;
  lineNumber: number | null;
  pageNumber: number | null;
  rectangles?: SearchVisualRect[];
}

interface FileSpaceSearchPreview {
  fileId: string;
  sections: FileSpaceSearchPreviewSection[];
  matchCount: number;
  truncated: boolean;
  extractionStatus: string;
  sourceUpdatedAt?: number;
  sourceSizeBytes?: number;
}

interface FileSpaceSearchPanelProps {
  open: boolean;
  files: FileSpaceSearchFile[];
  folders: FileSpaceSearchFolder[];
  scopes: FileSpaceSearchScope[];
  onOpen: () => void;
  onClose: () => void;
  onOpenFile: (file: FileSpaceSearchFile, context: FileOpenSearchContext | null) => void;
}

function matchesSearch(value: string, query: string) {
  return value.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
}

function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function fileExtension(name: string) {
  return name.includes(".") ? name.split(".").pop()?.toLocaleUpperCase() ?? "" : "";
}

function fileIcon(file: FileSpaceSearchFile) {
  const mime = file.mimeType ?? "";
  const extension = fileExtension(file.name).toLocaleLowerCase();
  if (mime.startsWith("image/")) return FileImage;
  if (mime.includes("spreadsheet") || mime.includes("excel") || ["csv", "xls", "xlsx"].includes(extension)) {
    return FileSpreadsheet;
  }
  if (
    mime.startsWith("text/")
    || mime.includes("pdf")
    || mime.includes("word")
    || ["md", "markdown", "txt", "doc", "docx", "pdf", "ppt", "pptx", "csr", "p12", "cer"].includes(extension)
  ) return FileText;
  return File;
}

function relevancePercent(similarity: number) {
  return Math.round(Math.min(1, Math.max(0, similarity)) * 100);
}

function HighlightedText({ text, query, enabled = true }: {
  text: string;
  query: string;
  enabled?: boolean;
}) {
  if (!enabled) return text;
  return splitSearchText(text, query).map((part, index) => (
    part.highlighted
      ? <mark key={`${index}-${part.text}`}>{part.text}</mark>
      : <span key={`${index}-${part.text}`}>{part.text}</span>
  ));
}

export function FileSpaceSearchPanel({
  open,
  files,
  folders,
  scopes,
  onOpen,
  onClose,
  onOpenFile,
}: FileSpaceSearchPanelProps) {
  const invoke = useWorkspaceInvoke();
  const applicationExtension = useApplicationExtension();
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [searchMatches, setSearchMatches] = useState<FileSpaceSearchMatch[]>([]);
  const [resolvedFiles, setResolvedFiles] = useState<FileSpaceSearchFile[]>([]);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const [preview, setPreview] = useState<FileSpaceSearchPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewFailed, setPreviewFailed] = useState(false);
  const [activeHitIndex, setActiveHitIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const resultRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const searchSequenceRef = useRef(0);
  const previewSequenceRef = useRef(0);
  const filesRef = useRef(files);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const restoreFocusRef = useRef(true);
  const restoreFocusPendingRef = useRef(false);
  const wasOpenRef = useRef(false);
  const presence = usePresence(open, 160);
  const shortcut = globalSearchShortcutLabel();

  const folderPaths = useMemo(
    () => new Map(folders.map((folder) => [folder.id, folder.relativePath])),
    [folders],
  );
  const filesById = useMemo(() => new Map(
    [...files, ...resolvedFiles].map((file) => [file.id, file]),
  ), [files, resolvedFiles]);
  const matchesByFileId = useMemo(
    () => new Map(searchMatches.map((match) => [match.fileId, match])),
    [searchMatches],
  );
  const results = useMemo(
    () => searchMatches.flatMap((match) => {
      const file = filesById.get(match.fileId);
      return file ? [file] : [];
    }),
    [filesById, searchMatches],
  );
  const activeFile = results[activeIndex] ?? null;
  const activeMatch = activeFile ? matchesByFileId.get(activeFile.id) ?? null : null;
  const activeSection = preview?.sections[activeHitIndex] ?? null;
  const showContentPreview = activeMatch?.contentMatch === true;
  const visualKind = activeFile ? visualPreviewKind(activeFile.name) : null;

  const closePanel = (restoreFocus = true) => {
    restoreFocusRef.current = restoreFocus;
    onClose();
  };

  const openFile = (file: FileSpaceSearchFile) => {
    restoreFocusRef.current = false;
    const match = matchesByFileId.get(file.id);
    const section = preview?.fileId === file.id ? activeSection : null;
    onOpenFile(file, match?.contentMatch ? {
      fileId: file.id,
      query: query.trim(),
      lineNumber: section?.lineNumber ?? null,
      pageNumber: section?.pageNumber ?? null,
      matchIndex: preview?.fileId === file.id ? activeHitIndex : 0,
    } : null);
  };

  useEffect(() => {
    filesRef.current = files;
  }, [files]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (applicationExtension?.active) return;
      if (event.target instanceof Element && event.target.closest("[data-help-center]")) return;
      if (!isGlobalSearchShortcut(event)) return;
      const activeModal = document.querySelector<HTMLElement>('[aria-modal="true"]');
      if (activeModal && !activeModal.classList.contains("file-space-global-search-panel")) return;
      event.preventDefault();
      event.stopPropagation();
      if (open) {
        inputRef.current?.focus();
        return;
      }
      onOpen();
    };
    window.addEventListener("keydown", handleShortcut, true);
    return () => window.removeEventListener("keydown", handleShortcut, true);
  }, [onOpen, open, applicationExtension?.active]);

  useEffect(() => {
    if (open && !wasOpenRef.current) {
      returnFocusRef.current = document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
      restoreFocusRef.current = true;
      wasOpenRef.current = true;
    }
    if (open && presence.mounted) {
      window.requestAnimationFrame(() => inputRef.current?.focus());
      return;
    }
    if (!open && wasOpenRef.current) {
      wasOpenRef.current = false;
      restoreFocusPendingRef.current = true;
    }
    if (!open && !presence.mounted && restoreFocusPendingRef.current) {
      restoreFocusPendingRef.current = false;
      const target = returnFocusRef.current?.isConnected
        ? returnFocusRef.current
        : document.querySelector<HTMLElement>(".file-space-file-card.is-selected > button, .file-space-search-trigger");
      if (restoreFocusRef.current && target) window.requestAnimationFrame(() => target.focus());
      returnFocusRef.current = null;
    }
  }, [open, presence.mounted]);

  useEffect(() => {
    if (!open) return undefined;
    const normalized = query.trim();
    const sequence = searchSequenceRef.current + 1;
    searchSequenceRef.current = sequence;
    setActiveIndex(0);
    setFailed(false);
    if (!normalized) {
      setSearchMatches([]);
      setResolvedFiles([]);
      setLoading(false);
      return undefined;
    }
    setLoading(true);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const matches = isTauri()
            ? await invoke<FileSpaceSearchMatch[]>("search_file_space_files", {
                request: { query: normalized, scopes },
              })
            : filesRef.current
                .filter((file) => (
                  (scopes.includes("name") && matchesSearch(file.name, normalized))
                  || (scopes.includes("tag") && file.tags.some((tag) => matchesSearch(tag, normalized)))
                ))
                .sort((left, right) => right.updatedAt - left.updatedAt)
                .map((file) => ({
                  fileId: file.id,
                  lexicalMatch: true,
                  semanticSimilarity: null,
                }));
          if (sequence !== searchSequenceRef.current) return;
          const boundedMatches = matches.slice(0, 320);
          if (isTauri() && boundedMatches.length > 0) {
            const page = await invoke<FileSpaceSearchFilePage>("list_file_space_files", {
              request: {
                folderId: null,
                sort: "updatedDesc",
                typeFilter: "all",
                tagFilter: null,
                updatedAfter: null,
                matchIds: boundedMatches.map((match) => match.fileId),
                cursor: null,
                limit: 320,
              },
            });
            if (sequence !== searchSequenceRef.current) return;
            setResolvedFiles(page.files);
          } else {
            setResolvedFiles([]);
          }
          setSearchMatches(boundedMatches);
        } catch {
          if (sequence !== searchSequenceRef.current) return;
          setSearchMatches([]);
          setResolvedFiles([]);
          setFailed(true);
        } finally {
          if (sequence === searchSequenceRef.current) setLoading(false);
        }
      })();
    }, 150);
    return () => window.clearTimeout(timer);
  }, [open, query, scopes]);

  useEffect(() => {
    const sequence = previewSequenceRef.current + 1;
    previewSequenceRef.current = sequence;
    setActiveHitIndex(0);
    setPreviewFailed(false);
    if (!open || !activeFile || !activeMatch || !showContentPreview || !query.trim()) {
      setPreview(null);
      setPreviewLoading(false);
      return undefined;
    }
    const semanticFallback = (): FileSpaceSearchPreview | null => activeMatch.snippet
      ? {
          fileId: activeFile.id,
          sections: [{ text: activeMatch.snippet, lineNumber: null, pageNumber: null }],
          matchCount: activeMatch.contentMatch ? 1 : 0,
          truncated: false,
          extractionStatus: "extracted",
        }
      : null;
    if (!isTauri()) {
      setPreview(semanticFallback());
      setPreviewLoading(false);
      return undefined;
    }
    setPreview(null);
    setPreviewLoading(true);
    void invoke<FileSpaceSearchPreview>("get_file_space_search_preview", {
      request: { fileId: activeFile.id, query: query.trim(), scopes },
    }).then((value) => {
      if (sequence !== previewSequenceRef.current) return;
      setPreview(value.sections.length > 0 || visualKind ? value : semanticFallback() ?? value);
    }).catch(() => {
      if (sequence !== previewSequenceRef.current) return;
      const fallback = semanticFallback();
      setPreview(fallback);
      setPreviewFailed(!fallback);
    }).finally(() => {
      if (sequence === previewSequenceRef.current) setPreviewLoading(false);
    });
    return () => {
      if (previewSequenceRef.current === sequence) previewSequenceRef.current += 1;
    };
  }, [activeFile, activeMatch, invoke, open, query, scopes, showContentPreview]);

  useEffect(() => {
    if (!open) return undefined;
    const handlePanelKeyboard = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        closePanel();
        return;
      }
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        if (results.length === 0) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        setActiveIndex((current) => {
          const next = event.key === "ArrowDown"
            ? (current + 1) % results.length
            : (current - 1 + results.length) % results.length;
          window.requestAnimationFrame(() => resultRefs.current[next]?.scrollIntoView({ block: "nearest" }));
          return next;
        });
        return;
      }
      if ((event.key === "PageDown" || event.key === "PageUp") && (preview?.sections.length ?? 0) > 1) {
        event.preventDefault();
        event.stopImmediatePropagation();
        setActiveHitIndex((current) => event.key === "PageDown"
          ? (current + 1) % preview!.sections.length
          : (current - 1 + preview!.sections.length) % preview!.sections.length);
        return;
      }
      // Let focused preview controls (including Retry) receive native Enter
      // activation instead of opening the selected file behind the control.
      if (event.key === "Enter" && event.target instanceof Element
        && event.target.closest(".file-space-global-search-preview button")) return;
      if (event.key === "Enter" && results[activeIndex]) {
        event.preventDefault();
        event.stopImmediatePropagation();
        openFile(results[activeIndex]);
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(panelRef.current?.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])',
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
    };
    window.addEventListener("keydown", handlePanelKeyboard, true);
    return () => window.removeEventListener("keydown", handlePanelKeyboard, true);
  }, [activeIndex, onClose, onOpenFile, open, preview, results]);

  if (!presence.mounted) return null;

  return createPortal(
    <div
      className="file-space-global-search-backdrop"
      data-state={presence.state}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) closePanel();
      }}
    >
      <div
        ref={panelRef}
        className={`file-space-global-search-panel${showContentPreview ? " has-content-preview" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby="file-space-global-search-title"
        data-state={presence.state}
      >
        <h2 id="file-space-global-search-title" className="file-space-global-search-title">
          {t("fileSpace.globalSearch.title")}
        </h2>
        <div className="file-space-global-search-input-row">
          {loading ? <LoaderCircle className="is-spinning" size={20} /> : <Search size={20} />}
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("fileSpace.globalSearch.placeholder")}
            role="combobox"
            aria-expanded={Boolean(query.trim())}
            aria-controls="file-space-global-search-results"
            aria-activedescendant={results[activeIndex] ? `file-space-global-search-result-${results[activeIndex].id}` : undefined}
            aria-autocomplete="list"
            autoComplete="off"
            spellCheck={false}
          />
          {query ? (
            <button type="button" onClick={() => setQuery("")} aria-label={t("fileSpace.globalSearch.clear")}>
              <X size={15} />
            </button>
          ) : <kbd>{shortcut}</kbd>}
        </div>

        <div className="file-space-global-search-body">
          {!query.trim() ? (
            <div className="file-space-global-search-state">
              <span><FolderOpen size={24} /></span>
              <strong>{t("fileSpace.globalSearch.startTitle")}</strong>
              <p>{t("fileSpace.globalSearch.startDescription")}</p>
            </div>
          ) : loading ? (
            <div className="file-space-global-search-state" role="status">
              <LoaderCircle className="is-spinning" size={22} />
              <strong>{t("fileSpace.globalSearch.searching")}</strong>
            </div>
          ) : failed ? (
            <div className="file-space-global-search-state" role="alert">
              <strong>{t("fileSpace.globalSearch.errorTitle")}</strong>
              <p>{t("fileSpace.globalSearch.errorDescription")}</p>
            </div>
          ) : results.length === 0 ? (
            <div className="file-space-global-search-state" role="status">
              <Search size={22} />
              <strong>{t("fileSpace.globalSearch.noResultsTitle")}</strong>
              <p>{t("fileSpace.globalSearch.noResultsDescription")}</p>
            </div>
          ) : (
            <div className={`file-space-global-search-content${showContentPreview ? " has-content-preview" : ""}`}>
              <section className="file-space-global-search-result-column">
                <div className="file-space-global-search-results-heading">
                  <span>{t("fileSpace.globalSearch.resultCount", { count: results.length })}</span>
                </div>
                <div id="file-space-global-search-results" className="file-space-global-search-results" role="listbox">
                  {results.map((file, index) => {
                  const Icon = fileIcon(file);
                  const match = matchesByFileId.get(file.id);
                  const extension = fileExtension(file.name) || t("fileSpace.globalSearch.fileType");
                  const location = file.folderId
                    ? folderPaths.get(file.folderId) ?? file.relativePath.split("/").slice(0, -1).join("/")
                    : t("fileSpace.globalSearch.rootLocation");
                  return (
                    <button
                      ref={(element) => { resultRefs.current[index] = element; }}
                      id={`file-space-global-search-result-${file.id}`}
                      className={index === activeIndex ? "is-active" : ""}
                      type="button"
                      role="option"
                      aria-selected={index === activeIndex}
                      key={file.id}
                      onFocus={() => setActiveIndex(index)}
                      onClick={() => setActiveIndex(index)}
                      onDoubleClick={() => openFile(file)}
                    >
                      <span className="file-space-global-search-file-icon"><Icon size={19} /></span>
                      <span className="file-space-global-search-result-copy">
                        <strong><HighlightedText text={file.name} query={query} enabled={Boolean(match?.lexicalMatch)} /></strong>
                        <small><FolderOpen size={12} />{location}</small>
                        {match?.snippet ? (
                          <span className="file-space-global-search-result-snippet">
                            <HighlightedText text={match.snippet} query={query} enabled={Boolean(match.contentMatch)} />
                          </span>
                        ) : null}
                      </span>
                      <span className="file-space-global-search-result-meta">
                        <small>{extension} · {formatFileSize(file.sizeBytes)}</small>
                        {match ? (
                          <small
                            className="file-space-global-search-match-meta"
                            title={t("fileSpace.globalSearch.relevanceDescription")}
                          >
                            {match.contentMatch
                              ? t("fileSpace.globalSearch.contentMatch")
                              : match.semanticSimilarity !== null
                              ? t(match.lexicalMatch
                                  ? "fileSpace.globalSearch.hybridRelevance"
                                  : "fileSpace.globalSearch.semanticRelevance", {
                                  score: relevancePercent(match.semanticSimilarity),
                                })
                              : t("fileSpace.globalSearch.keywordMatch")}
                            {file.versionCount > 1
                              ? <> · {t("fileSpace.globalSearch.versionCount", { count: file.versionCount })}</>
                              : null}
                          </small>
                        ) : file.versionCount > 1 ? (
                          <small>{t("fileSpace.globalSearch.versionCount", { count: file.versionCount })}</small>
                        ) : null}
                      </span>
                    </button>
                  );
                  })}
                </div>
              </section>

              {showContentPreview ? <section className="file-space-global-search-preview" aria-live="polite">
                <header className="file-space-global-search-preview-toolbar">
                  <div>
                    <strong>{activeFile?.name}</strong>
                    <small>{activeFile?.relativePath}</small>
                  </div>
                  {preview && preview.sections.length > 1 ? (
                    <div className="file-space-global-search-hit-navigation">
                      <button
                        type="button"
                        onClick={() => setActiveHitIndex((current) => (
                          current - 1 + preview.sections.length
                        ) % preview.sections.length)}
                        aria-label={t("fileSpace.globalSearch.previousMatch")}
                      >
                        <ChevronUp size={15} />
                      </button>
                      <span>{t("fileSpace.globalSearch.matchPosition", {
                        current: activeHitIndex + 1,
                        count: preview.sections.length,
                      })}</span>
                      <button
                        type="button"
                        onClick={() => setActiveHitIndex((current) => (
                          current + 1
                        ) % preview.sections.length)}
                        aria-label={t("fileSpace.globalSearch.nextMatch")}
                      >
                        <ChevronDown size={15} />
                      </button>
                    </div>
                  ) : null}
                  {activeFile ? (
                    <button className="file-space-global-search-open-file" type="button" onClick={() => openFile(activeFile)}>
                      <ExternalLink size={14} />
                      {t("fileSpace.globalSearch.openFile")}
                    </button>
                  ) : null}
                </header>
                <div className="file-space-global-search-preview-body">
                  {activeFile && visualKind ? (
                    <SearchVisualPreview key={`${activeFile.id}:${activeFile.updatedAt}`} file={activeFile} kind={visualKind}
                      sections={preview?.fileId === activeFile.id ? preview.sections : []} activeIndex={activeHitIndex}
                      ready={!previewLoading && preview?.fileId === activeFile.id
                        && preview.sourceUpdatedAt === activeFile.updatedAt && preview.sourceSizeBytes === activeFile.sizeBytes} />
                  ) : previewLoading ? (
                    <div className="file-space-global-search-preview-state" role="status">
                      <LoaderCircle className="is-spinning" size={20} />
                      <span>{t("fileSpace.globalSearch.loadingPreview")}</span>
                    </div>
                  ) : previewFailed ? (
                    <div className="file-space-global-search-preview-state" role="alert">
                      <span>{t("fileSpace.globalSearch.previewError")}</span>
                    </div>
                  ) : activeSection ? (
                    <article className="file-space-global-search-preview-document">
                      <div className="file-space-global-search-preview-location">
                        {activeSection.pageNumber
                          ? t("fileSpace.globalSearch.pageLocation", { page: activeSection.pageNumber })
                          : activeSection.lineNumber
                            ? t("fileSpace.globalSearch.lineLocation", { line: activeSection.lineNumber })
                            : activeMatch?.contentMatch
                              ? t("fileSpace.globalSearch.contentLocation")
                              : t("fileSpace.globalSearch.semanticLocation")}
                      </div>
                      <p>
                        <HighlightedText
                          text={activeSection.text}
                          query={query}
                          enabled={Boolean(activeMatch?.contentMatch)}
                        />
                      </p>
                      {preview?.truncated ? (
                        <small>{t("fileSpace.globalSearch.moreMatches", { count: preview.matchCount })}</small>
                      ) : null}
                    </article>
                  ) : (
                    <div className="file-space-global-search-preview-state">
                      <span>{t("fileSpace.globalSearch.previewUnavailable")}</span>
                    </div>
                  )}
                </div>
              </section> : null}
            </div>
          )}
        </div>

        <footer className="file-space-global-search-footer">
          <span><kbd>↑</kbd><kbd>↓</kbd>{t("fileSpace.globalSearch.navigateHint")}</span>
          <span><kbd>↵</kbd>{t("fileSpace.globalSearch.openHint")}</span>
          <span><kbd>esc</kbd>{t("fileSpace.globalSearch.closeHint")}</span>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
