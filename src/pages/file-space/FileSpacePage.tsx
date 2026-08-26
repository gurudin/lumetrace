import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { toBlob } from "html-to-image";
import {
  ChevronDown,
  ChevronRight,
  Check,
  Clock3,
  Copy,
  File,
  FileImage,
  FileSpreadsheet,
  FileText,
  Filter,
  Folder,
  FolderPlus,
  FolderOpen,
  HardDrive,
  LoaderCircle,
  Minus,
  Pencil,
  Plus,
  Search,
  ShieldCheck,
  Tag,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { usePresence } from "../../shared/ui/usePresence";
import {
  calculateJustifiedFileLayout,
  justifiedFileLayoutsEqual,
  reorderFileIdsForDraggedCard,
  type JustifiedFileLayout,
} from "./fileJustifiedLayout";

interface FileSpaceFolderRecord {
  id: string;
  parentId: string | null;
  name: string;
  relativePath: string;
  manualOrder: number;
  createdAt: number;
  updatedAt: number;
}

interface FileSpaceFileRecord {
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

interface TaskFileVersionRecord {
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

interface TaskFileEventRecord {
  id: string;
  versionId: string | null;
  eventType: string;
  actor: "task" | "user";
  details: Record<string, unknown>;
  createdAt: number;
}

interface TaskFileTimelineRecord {
  fileId: string;
  logicalKey: string;
  currentVersionId: string;
  versions: TaskFileVersionRecord[];
  events: TaskFileEventRecord[];
}

interface FileSpaceSnapshot {
  rootPath: string | null;
  rootName: string | null;
  rootStatus: "unconfigured" | "ready" | "missing" | "notDirectory" | "notWritable";
  folders: FileSpaceFolderRecord[];
  files: FileSpaceFileRecord[];
}

type TypeFilter = "all" | "document" | "image" | "sheet";
type SearchScope = "name" | "content" | "tag";
type TimeFilter = "all" | "today" | "week" | "month" | "year";
type SortOption = "manual" | "updatedDesc" | "updatedAsc" | "nameAsc" | "nameDesc" | "sizeDesc" | "typeAsc";
type FolderContextMenu = { folderId: string; x: number; y: number };
type FileContextMenu = { fileId: string; x: number; y: number };
type FileSpaceBusyAction = "configure" | "create" | "rename" | "delete" | "import" | "fileAction" | "reorder" | "moveFolder" | "moveFile";
type RootSetupMode = "new" | "import";
type FileSpaceOperation = {
  token: symbol;
  action: FileSpaceBusyAction;
  generation: number;
};
type FileSpaceImportPhase = "scanning" | "importing" | "cancelling" | "completed" | "cancelled" | "failed";
interface FileSpaceImportFeedback {
  requestId: string;
  phase: FileSpaceImportPhase;
  processed: number;
  total: number;
  currentName: string | null;
  error?: string;
}

interface FileMoveFeedback {
  fileName: string;
  destinationName: string;
  status: "moved" | "unchanged";
}

interface BrowserDroppedFile {
  file: File;
  relativePath: string;
}

interface FileSpaceDroppedFilePayload {
  relativePath: string;
  bytes: number[];
}

interface FileDragGesture {
  fileId: string;
  pointerId: number;
  startX: number;
  startY: number;
  offsetX: number;
  offsetY: number;
  cardWidth: number;
  cardHeight: number;
  target: HTMLButtonElement;
  orderedIds: string[];
  phase: "pending" | "reordering";
}

interface InternalFileDrag {
  fileId: string;
  pointerId: number;
  pointerX: number;
  pointerY: number;
  offsetX: number;
  offsetY: number;
  cardWidth: number;
  cardHeight: number;
  originalIds: string[];
  orderedIds: string[];
  folderTarget: FileFolderDropTarget | null;
}

interface FileFolderDropTarget {
  folderId: string | null;
  targetId: string | null;
  kind: "folder" | "root";
  isCurrent: boolean;
}

type FolderDropMode = "before" | "inside" | "after" | "root";

interface FolderDropTarget {
  mode: FolderDropMode;
  targetId: string | null;
  parentId: string | null;
  orderedSiblingIds: string[];
}

interface FolderDragGesture {
  folderId: string;
  pointerId: number;
  startX: number;
  startY: number;
  offsetX: number;
  offsetY: number;
  rowWidth: number;
  rowHeight: number;
  phase: "pending" | "dragging";
}

interface InternalFolderDrag {
  folderId: string;
  pointerId: number;
  pointerX: number;
  pointerY: number;
  offsetX: number;
  offsetY: number;
  rowWidth: number;
  rowHeight: number;
  target: FolderDropTarget | null;
}

const emptySnapshot: FileSpaceSnapshot = {
  rootPath: null,
  rootName: null,
  rootStatus: "unconfigured",
  folders: [],
  files: [],
};
const currentFolderStorageKey = "lumetrace.file-space.current-folder";
const previewSizeStorageKey = "lumetrace.file-space.preview-size";
const sortOptionStorageKey = "lumetrace.file-space.sort-option";
const folderDragThreshold = 5;
const previewSizeMin = 120;
const previewSizeMax = 260;
const previewSizeStep = 10;
const previewSizeDefault = 155;
const fileLayoutHorizontalGap = 15;
const fileLayoutVerticalGap = 18;
const filePreviewDetailsGap = 9;
const fileDragThreshold = 6;
const fileDragWindowExitMargin = 1;
const fileDragPreviewMaxSize = 160;

function pngDataUrlBytes(dataUrl: string) {
  const encoded = dataUrl.split(",")[1];
  if (!encoded) return null;
  const binary = window.atob(encoded);
  return Array.from(binary, (character) => character.charCodeAt(0));
}

function imageDragPreviewBytes(target: HTMLButtonElement) {
  const image = target.querySelector<HTMLImageElement>(".file-space-file-art.has-preview img");
  if (!image?.complete || image.naturalWidth <= 0 || image.naturalHeight <= 0) return null;
  const scale = Math.min(
    1,
    fileDragPreviewMaxSize / Math.max(image.naturalWidth, image.naturalHeight),
  );
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
  const context = canvas.getContext("2d");
  if (!context) return null;
  try {
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return pngDataUrlBytes(canvas.toDataURL("image/png"));
  } catch {
    return null;
  }
}

async function fileDragPreviewBytes(target: HTMLButtonElement) {
  const imagePreview = imageDragPreviewBytes(target);
  if (imagePreview) return imagePreview;

  const artwork = target.querySelector<HTMLElement>(".file-space-file-art");
  if (!artwork) return null;
  const { width, height } = artwork.getBoundingClientRect();
  if (width <= 0 || height <= 0) return null;
  const scale = Math.min(1, fileDragPreviewMaxSize / Math.max(width, height));

  try {
    const blob = await toBlob(artwork, {
      canvasWidth: Math.max(1, Math.round(width * scale)),
      canvasHeight: Math.max(1, Math.round(height * scale)),
      pixelRatio: 1,
      skipFonts: true,
    });
    return blob ? Array.from(new Uint8Array(await blob.arrayBuffer())) : null;
  } catch {
    return null;
  }
}

function createImportRequestId() {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `import-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function entryRelativePath(entry: FileSystemEntry) {
  return entry.fullPath.replace(/^\/+/, "") || entry.name;
}

function readBrowserFileEntry(entry: FileSystemFileEntry) {
  return new Promise<File>((resolve, reject) => entry.file(resolve, reject));
}

function readBrowserDirectoryEntries(reader: FileSystemDirectoryReader) {
  return new Promise<FileSystemEntry[]>((resolve, reject) => {
    const entries: FileSystemEntry[] = [];
    const readBatch = () => {
      reader.readEntries((batch) => {
        if (batch.length === 0) {
          resolve(entries);
          return;
        }
        entries.push(...batch);
        readBatch();
      }, reject);
    };
    readBatch();
  });
}

async function collectBrowserEntryFiles(
  entry: FileSystemEntry,
  result: BrowserDroppedFile[],
) {
  if (entry.isFile) {
    const file = await readBrowserFileEntry(entry as FileSystemFileEntry);
    result.push({ file, relativePath: entryRelativePath(entry) });
    return;
  }
  if (!entry.isDirectory) return;
  const children = await readBrowserDirectoryEntries(
    (entry as FileSystemDirectoryEntry).createReader(),
  );
  for (const child of children) {
    await collectBrowserEntryFiles(child, result);
  }
}

async function collectBrowserDroppedFiles(dataTransfer: DataTransfer) {
  const entries = Array.from(dataTransfer.items)
    .filter((item) => item.kind === "file")
    .map((item) => item.webkitGetAsEntry())
    .filter((entry): entry is FileSystemEntry => Boolean(entry));
  if (entries.length > 0) {
    const files: BrowserDroppedFile[] = [];
    for (const entry of entries) await collectBrowserEntryFiles(entry, files);
    return files;
  }
  return Array.from(dataTransfer.files).map((file) => ({
    file,
    relativePath: file.webkitRelativePath || file.name,
  }));
}

function localPathsFromUriList(dataTransfer: DataTransfer) {
  return dataTransfer
    .getData("text/uri-list")
    .split(/\r?\n/)
    .map((value) => value.trim())
    .filter((value) => value && !value.startsWith("#"))
    .flatMap((value) => {
      try {
        const url = new URL(value);
        return url.protocol === "file:" ? [decodeURIComponent(url.pathname)] : [];
      } catch {
        return [];
      }
    });
}

function supportsExternalFileDrop(dataTransfer: DataTransfer) {
  const types = Array.from(dataTransfer.types);
  return types.includes("Files") || types.includes("text/uri-list");
}

const textPreviewMimeTypes = new Set([
  "application/json",
  "application/xml",
  "application/javascript",
  "image/svg+xml",
]);

function supportsTextPreview(version: TaskFileVersionRecord) {
  if (version.mimeType?.startsWith("text/")) return true;
  if (version.mimeType && textPreviewMimeTypes.has(version.mimeType)) return true;
  const extension = version.name.split(".").pop()?.toLowerCase();
  return Boolean(extension && ["md", "markdown", "txt", "csv", "json", "xml", "yaml", "yml", "html", "css", "js", "ts", "tsx", "jsx", "svg"].includes(extension));
}

function eventTranslationKey(eventType: string) {
  const knownTypes = new Set([
    "generated",
    "modified",
    "renamed",
    "deleted",
    "restored",
    "current_version_changed",
  ]);
  return knownTypes.has(eventType) ? `fileSpace.timeline.events.${eventType}` : null;
}

function clampPreviewSize(value: number) {
  return Math.min(previewSizeMax, Math.max(previewSizeMin, value));
}

function storedPreviewSize() {
  const value = Number(window.localStorage.getItem(previewSizeStorageKey));
  return Number.isFinite(value) && value > 0
    ? clampPreviewSize(value)
    : previewSizeDefault;
}

const sortOptions: SortOption[] = [
  "manual",
  "updatedDesc",
  "updatedAsc",
  "nameAsc",
  "nameDesc",
  "sizeDesc",
  "typeAsc",
];

function storedSortOption(): SortOption {
  const value = window.localStorage.getItem(sortOptionStorageKey) as SortOption | null;
  return value && sortOptions.includes(value) ? value : "updatedDesc";
}

function sameOrder(left: string[], right: string[]) {
  return left.length === right.length && left.every((id, index) => id === right[index]);
}

function pointerReachedWindowEdge(clientX: number, clientY: number) {
  return clientX <= fileDragWindowExitMargin
    || clientY <= fileDragWindowExitMargin
    || clientX >= window.innerWidth - fileDragWindowExitMargin
    || clientY >= window.innerHeight - fileDragWindowExitMargin;
}

function reorderedIdsAtPointer(
  grid: HTMLElement,
  orderedIds: string[],
  draggedId: string,
  clientX: number,
  clientY: number,
  offsetX: number,
  offsetY: number,
  cardWidth: number,
  cardHeight: number,
) {
  const withoutDragged = orderedIds.filter((id) => id !== draggedId);
  const candidates = withoutDragged.flatMap((id) => {
    const card = grid.querySelector<HTMLElement>(`.file-space-file-card[data-file-id="${CSS.escape(id)}"]`);
    if (!card) return [];
    const rect = card.getBoundingClientRect();
    return [{
      id,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
    }];
  });
  return reorderFileIdsForDraggedCard({
    orderedIds,
    draggedId,
    candidates,
    draggedCenterX: clientX - offsetX + cardWidth / 2,
    draggedCenterY: clientY - offsetY + cardHeight / 2,
  });
}

function optimisticManualOrder(snapshot: FileSpaceSnapshot, orderedIds: string[]) {
  const requested = new Set(orderedIds);
  const globallyOrdered = [...snapshot.files].sort((left, right) => (
    left.manualOrder - right.manualOrder
    || left.createdAt - right.createdAt
    || left.id.localeCompare(right.id)
  ));
  const slots = globallyOrdered
    .map((file, index) => requested.has(file.id) ? index : -1)
    .filter((index) => index >= 0);
  if (slots.length !== orderedIds.length) return snapshot;
  slots.forEach((slot, index) => {
    const file = snapshot.files.find((item) => item.id === orderedIds[index]);
    if (file) globallyOrdered[slot] = file;
  });
  const positions = new Map(globallyOrdered.map((file, index) => [file.id, index]));
  return {
    ...snapshot,
    files: snapshot.files.map((file) => ({
      ...file,
      manualOrder: positions.get(file.id) ?? file.manualOrder,
    })),
  };
}

function createVisualFixture(): FileSpaceSnapshot {
  const now = Date.now();
  const folders: FileSpaceFolderRecord[] = [
    { id: "fixture-product", parentId: null, name: "Clipboard X", relativePath: "Clipboard X", manualOrder: 0, createdAt: now, updatedAt: now },
    { id: "fixture-research", parentId: "fixture-product", name: "市场调研", relativePath: "Clipboard X/市场调研", manualOrder: 0, createdAt: now, updatedAt: now },
    { id: "fixture-comments", parentId: "fixture-research", name: "公开评论", relativePath: "Clipboard X/市场调研/公开评论", manualOrder: 0, createdAt: now, updatedAt: now },
    { id: "fixture-competitors", parentId: "fixture-research", name: "竞品资料", relativePath: "Clipboard X/市场调研/竞品资料", manualOrder: 1, createdAt: now, updatedAt: now },
    { id: "fixture-plan", parentId: "fixture-product", name: "产品方案", relativePath: "Clipboard X/产品方案", manualOrder: 1, createdAt: now, updatedAt: now },
    { id: "fixture-promotion", parentId: "fixture-product", name: "推广资料", relativePath: "Clipboard X/推广资料", manualOrder: 2, createdAt: now, updatedAt: now },
  ];
  const extensions = ["png", "pdf", "html", "xlsx", "csv", "docx", "txt", "md", "zip", "zipx", "7z", "rar", "tar"];
  const mimeTypes = [
    "image/png",
    "application/pdf",
    "text/html",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "text/csv",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "text/plain",
    "text/markdown",
    "application/zip",
    "application/zip",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-tar",
  ];
  const files = Array.from({ length: 25 }, (_, index): FileSpaceFileRecord => ({
    id: `fixture-file-${index}`,
    folderId: "fixture-research",
    name: `文件空间真实密度测试资料-${String(index + 1).padStart(2, "0")}.${extensions[index % extensions.length]}`,
    relativePath: `Clipboard X/市场调研/fixture-${index}.${extensions[index % extensions.length]}`,
    mimeType: mimeTypes[index % mimeTypes.length],
    sizeBytes: (index + 2) * 91_000,
    sourceKind: index % 4 === 0 ? "task_artifact" : "user_import",
    manualOrder: index,
    currentVersion: index % 4 === 0 ? (index % 3) + 1 : null,
    versionCount: index % 4 === 0 ? (index % 3) + 1 : 0,
    tags: index % 3 === 0 ? ["调研", "Clipboard"] : index % 5 === 0 ? ["待审核"] : [],
    createdAt: now - index * 86_400_000,
    updatedAt: now - index * 86_400_000,
  }));
  return {
    rootPath: "/Users/example/Documents/LumeTrace",
    rootName: "LumeTrace",
    rootStatus: "ready",
    folders,
    files,
  };
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function waitForCommittedPaint() {
  return new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => window.requestAnimationFrame(() => resolve()));
  });
}

function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function fileCategory(file: FileSpaceFileRecord): Exclude<TypeFilter, "all"> | "other" {
  const mime = file.mimeType ?? "";
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (mime.startsWith("image/")) return "image";
  if (
    mime.includes("spreadsheet") ||
    mime.includes("excel") ||
    ["csv", "xls", "xlsx"].includes(extension)
  ) return "sheet";
  if (
    mime.startsWith("text/") ||
    mime.includes("pdf") ||
    mime.includes("word") ||
    ["md", "txt", "doc", "docx", "pdf", "ppt", "pptx"].includes(extension)
  ) return "document";
  return "other";
}

function fileIcon(file: FileSpaceFileRecord) {
  const category = fileCategory(file);
  if (category === "image") return FileImage;
  if (category === "sheet") return FileSpreadsheet;
  if (category === "document") return FileText;
  return File;
}

type FileArtworkFormat = "md" | "pdf" | "html" | "xlsx" | "csv" | "docx" | "txt" | "archive";
type ArchiveArtworkExtension = "zip" | "zipx" | "7z" | "rar" | "tar";

const archiveArtworkExtensions: ArchiveArtworkExtension[] = ["zip", "zipx", "7z", "rar", "tar"];

function fileExtension(file: FileSpaceFileRecord) {
  return file.name.includes(".") ? file.name.split(".").pop()?.toLowerCase() ?? "" : "";
}

function fileArchiveExtension(file: FileSpaceFileRecord): ArchiveArtworkExtension | null {
  const extension = fileExtension(file);
  return archiveArtworkExtensions.find((candidate) => candidate === extension) ?? null;
}

function fileArtworkFormat(file: FileSpaceFileRecord): FileArtworkFormat | null {
  const extension = fileExtension(file);
  if (fileArchiveExtension(file)) return "archive";
  return (["md", "pdf", "html", "xlsx", "csv", "docx", "txt"] as const).find((format) => format === extension) ?? null;
}

function defaultFilePreviewAspectRatio(file: FileSpaceFileRecord) {
  const format = fileArtworkFormat(file);
  if (format === "md") return 0.82;
  if (format === "pdf") return 0.76;
  if (format === "txt") return 0.9;
  if (format === "archive") return 0.736;
  if (fileCategory(file) === "image") return 4 / 3;
  return 155 / 108;
}

function fileArtworkTitle(file: FileSpaceFileRecord) {
  const extension = fileExtension(file);
  const basename = extension ? file.name.slice(0, -(extension.length + 1)) : file.name;
  return basename.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
}

function fixtureImageSource(file: FileSpaceFileRecord) {
  const index = Number(file.id.replace("fixture-file-", "")) || 0;
  const [width, height] = [
    [720, 1080],
    [1200, 760],
    [900, 900],
    [760, 1240],
    [1280, 880],
  ][index % 5];
  const colors = ["#9caaa4", "#b9a68f", "#939eae", "#aa9698", "#9aa788"];
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop stop-color="${colors[index % colors.length]}"/><stop offset="1" stop-color="#e5e2dc"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="${width * 0.72}" cy="${height * 0.27}" r="${Math.min(width, height) * 0.12}" fill="#fff" fill-opacity=".62"/><path d="M0 ${height * 0.78} L${width * 0.33} ${height * 0.45} L${width * 0.56} ${height * 0.67} L${width * 0.75} ${height * 0.52} L${width} ${height * 0.76} V${height} H0Z" fill="#35433e" fill-opacity=".34"/></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function filePreviewSource(file: FileSpaceFileRecord) {
  if (fileCategory(file) !== "image") return null;
  if (!isTauri()) return file.id.startsWith("fixture-file-") ? fixtureImageSource(file) : null;
  return convertFileSrc(file.id, "lumetrace-file-preview");
}

function FileArtwork({ file }: { file: FileSpaceFileRecord }) {
  const { t } = useTranslation();
  const Icon = fileIcon(file);
  const artworkFormat = fileArtworkFormat(file);
  const archiveExtension = fileArchiveExtension(file);
  const artworkTitle = fileArtworkTitle(file);
  const previewSource = filePreviewSource(file);
  const [previewFailed, setPreviewFailed] = useState(false);
  const showPreview = Boolean(previewSource) && !previewFailed;
  return (
    <span className={`file-space-file-art is-${fileCategory(file)}${artworkFormat ? ` is-format-${artworkFormat}` : ""}${showPreview ? " has-preview" : ""}`}>
      {showPreview ? (
        <img
          key={file.updatedAt}
          src={previewSource ?? undefined}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => setPreviewFailed(true)}
        />
      ) : artworkFormat ? (
        <span className={`file-space-format-art is-${artworkFormat}`}>
          {artworkFormat !== "archive" ? (
            <span className="file-space-format-kicker">
              {artworkFormat === "md" ? "MARKDOWN" : artworkFormat.toUpperCase()}
            </span>
          ) : null}
          {artworkFormat === "md" || artworkFormat === "pdf" ? (
            <span className="file-space-format-title">{artworkTitle}</span>
          ) : null}
          {artworkFormat === "html" ? (
            <span className="file-space-code-lines" aria-hidden="true">
              <i /><i /><i /><i /><i /><i />
            </span>
          ) : null}
          {artworkFormat === "xlsx" ? (
            <span className="file-space-sheet-grid" aria-hidden="true">
              {Array.from({ length: 15 }, (_, index) => <i key={index} />)}
            </span>
          ) : null}
          {artworkFormat === "csv" || artworkFormat === "docx" ? (
            <span className="file-space-csv-rows" aria-hidden="true">
              {Array.from({ length: 5 }, (_, row) => (
                <i key={row}><b /><b /><b /></i>
              ))}
            </span>
          ) : null}
          {artworkFormat === "archive" ? (
            <span className="file-space-archive-document" aria-hidden="true">
              <i className="file-space-archive-fold" />
              <span className="file-space-archive-track">
                <b />
                <span><i /><i /><i /><i /><i /></span>
              </span>
              <em>{archiveExtension?.toUpperCase()}</em>
            </span>
          ) : null}
          {artworkFormat === "txt" ? <span className="file-space-text-lines" aria-hidden="true" /> : null}
          {artworkFormat === "md" || artworkFormat === "pdf" ? (
            <span className="file-space-document-lines" aria-hidden="true" />
          ) : null}
        </span>
      ) : (
        <>
          <Icon size={42} strokeWidth={1.35} />
          <small>{file.name.split(".").pop()?.toUpperCase().slice(0, 5)}</small>
        </>
      )}
      {file.versionCount > 0 ? (
        <span className="file-space-file-version-count">
          {t("fileSpace.content.versionCount", { count: file.versionCount })}
        </span>
      ) : null}
    </span>
  );
}

function matchesSearch(value: string, query: string) {
  return value.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
}

function timeFilterStart(filter: TimeFilter, now = new Date()) {
  if (filter === "all") return null;
  const start = new Date(now);
  if (filter === "today") start.setHours(0, 0, 0, 0);
  if (filter === "week") start.setDate(start.getDate() - 7);
  if (filter === "month") start.setDate(start.getDate() - 30);
  if (filter === "year") start.setMonth(0, 1), start.setHours(0, 0, 0, 0);
  return start.getTime();
}

function compareFiles(left: FileSpaceFileRecord, right: FileSpaceFileRecord, sort: SortOption, locale: string) {
  if (sort === "manual") return left.manualOrder - right.manualOrder || left.id.localeCompare(right.id);
  if (sort === "updatedDesc") return right.updatedAt - left.updatedAt || left.name.localeCompare(right.name, locale);
  if (sort === "updatedAsc") return left.updatedAt - right.updatedAt || left.name.localeCompare(right.name, locale);
  if (sort === "nameDesc") return right.name.localeCompare(left.name, locale);
  if (sort === "sizeDesc") return right.sizeBytes - left.sizeBytes || left.name.localeCompare(right.name, locale);
  if (sort === "typeAsc") return fileExtension(left).localeCompare(fileExtension(right), locale) || left.name.localeCompare(right.name, locale);
  return left.name.localeCompare(right.name, locale);
}

export function FileSpacePage() {
  const { t, i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState<FileSpaceSnapshot>(emptySnapshot);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<FileSpaceBusyAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [setupCancelled, setSetupCancelled] = useState(false);
  const [rootSetupMode, setRootSetupMode] = useState<RootSetupMode | null>(null);
  const [currentFolderId, setCurrentFolderId] = useState<string | null>(() => (
    window.localStorage.getItem(currentFolderStorageKey)
  ));
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [searchScopes, setSearchScopes] = useState<SearchScope[]>(["name", "content", "tag"]);
  const [searchResult, setSearchResult] = useState<{ key: string; ids: Set<string> } | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [tagFilter, setTagFilter] = useState("all");
  const [timeFilter, setTimeFilter] = useState<TimeFilter>("all");
  const [sortOption, setSortOption] = useState<SortOption>(storedSortOption);
  const [previewSize, setPreviewSize] = useState(storedPreviewSize);
  const [fileJustifiedLayout, setFileJustifiedLayout] = useState<JustifiedFileLayout | null>(null);
  const [isFilterMenuOpen, setFilterMenuOpen] = useState(false);
  const [isCreateFolderOpen, setCreateFolderOpen] = useState(false);
  const [createParentId, setCreateParentId] = useState<string | null>(null);
  const [folderName, setFolderName] = useState("");
  const [renameFolderId, setRenameFolderId] = useState<string | null>(null);
  const [renameFolderName, setRenameFolderName] = useState("");
  const [deleteFolderId, setDeleteFolderId] = useState<string | null>(null);
  const [folderContextMenu, setFolderContextMenu] = useState<FolderContextMenu | null>(null);
  const [fileContextMenu, setFileContextMenu] = useState<FileContextMenu | null>(null);
  const [renameFileId, setRenameFileId] = useState<string | null>(null);
  const [renameFileName, setRenameFileName] = useState("");
  const [deleteFileId, setDeleteFileId] = useState<string | null>(null);
  const [tagFileId, setTagFileId] = useState<string | null>(null);
  const [tagDraft, setTagDraft] = useState("");
  const [timeline, setTimeline] = useState<TaskFileTimelineRecord | null>(null);
  const [timelineFile, setTimelineFile] = useState<FileSpaceFileRecord | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [versionPreview, setVersionPreview] = useState<string | null>(null);
  const [timelineBusy, setTimelineBusy] = useState(false);
  const [isFileDragOver, setFileDragOver] = useState(false);
  const [internalFileDrag, setInternalFileDrag] = useState<InternalFileDrag | null>(null);
  const [internalFolderDrag, setInternalFolderDrag] = useState<InternalFolderDrag | null>(null);
  const [importFeedback, setImportFeedback] = useState<FileSpaceImportFeedback | null>(null);
  const [fileMoveFeedback, setFileMoveFeedback] = useState<FileMoveFeedback | null>(null);
  const folderNameRef = useRef<HTMLInputElement>(null);
  const renameFolderNameRef = useRef<HTMLInputElement>(null);
  const renameFileNameRef = useRef<HTMLInputElement>(null);
  const tagDraftRef = useRef<HTMLInputElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const filterMenuRef = useRef<HTMLDivElement>(null);
  const contentDropZoneRef = useRef<HTMLDivElement>(null);
  const fileGridRef = useRef<HTMLDivElement>(null);
  const folderTreeRef = useRef<HTMLDivElement>(null);
  const operationRef = useRef<FileSpaceOperation | null>(null);
  const lifecycleRef = useRef({ mounted: false, generation: 0 });
  const nativeDropBlockedRef = useRef(false);
  const externalDragDepthRef = useRef(0);
  const searchSequenceRef = useRef(0);
  const importRequestRef = useRef<string | null>(null);
  const fileDragGestureRef = useRef<FileDragGesture | null>(null);
  const internalFileDragRef = useRef<InternalFileDrag | null>(null);
  const folderDragGestureRef = useRef<FolderDragGesture | null>(null);
  const internalFolderDragRef = useRef<InternalFolderDrag | null>(null);
  const suppressFolderClickRef = useRef(false);
  const folderClickReleaseTimerRef = useRef<number | null>(null);
  const fileDragStartingRef = useRef(false);
  const fileDragReleaseTimerRef = useRef<number | null>(null);
  const createDialogPresence = usePresence(isCreateFolderOpen);
  const renameDialogPresence = usePresence(Boolean(renameFolderId));
  const deleteDialogPresence = usePresence(Boolean(deleteFolderId));
  const renameFileDialogPresence = usePresence(Boolean(renameFileId));
  const deleteFileDialogPresence = usePresence(Boolean(deleteFileId));
  const tagDialogPresence = usePresence(Boolean(tagFileId));
  const filterMenuPresence = usePresence(isFilterMenuOpen, 140);

  const isChinese = i18n.resolvedLanguage?.startsWith("zh") ?? true;
  const locale = isChinese ? "zh-CN" : "en-US";
  const searchKey = `${query.trim()}\u0000${[...searchScopes].sort().join(",")}`;

  useEffect(() => {
    const normalized = query.trim();
    const sequence = searchSequenceRef.current + 1;
    searchSequenceRef.current = sequence;
    if (!normalized) {
      setSearchResult(null);
      setSearchLoading(false);
      return undefined;
    }
    setSearchLoading(true);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const ids = isTauri()
            ? await invoke<string[]>("search_file_space_files", {
                request: { query: normalized, scopes: searchScopes },
              })
            : snapshot.files
                .filter((file) => (
                  (searchScopes.includes("name") && matchesSearch(file.name, normalized))
                  || (searchScopes.includes("tag") && file.tags.some((tag) => matchesSearch(tag, normalized)))
                ))
                .map((file) => file.id);
          if (sequence !== searchSequenceRef.current) return;
          setSearchResult({ key: searchKey, ids: new Set(ids) });
        } catch (searchError) {
          if (sequence !== searchSequenceRef.current) return;
          setSearchResult({ key: searchKey, ids: new Set() });
          setError(`${t("fileSpace.errors.search")} ${errorText(searchError)}`);
        } finally {
          if (sequence === searchSequenceRef.current) setSearchLoading(false);
        }
      })();
    }, 180);
    return () => window.clearTimeout(timer);
  }, [query, searchKey, searchScopes, snapshot.files, t]);

  const updatePreviewSize = (value: number) => {
    const nextSize = clampPreviewSize(value);
    setPreviewSize(nextSize);
    window.localStorage.setItem(previewSizeStorageKey, String(nextSize));
  };

  const updateSortOption = (value: SortOption) => {
    setSortOption(value);
    window.localStorage.setItem(sortOptionStorageKey, value);
  };

  const updateInternalFileDrag = (value: InternalFileDrag | null) => {
    internalFileDragRef.current = value;
    setInternalFileDrag(value);
  };

  const updateInternalFolderDrag = (value: InternalFolderDrag | null) => {
    internalFolderDragRef.current = value;
    setInternalFolderDrag(value);
  };

  const beginOperation = (action: FileSpaceBusyAction) => {
    if (!lifecycleRef.current.mounted || operationRef.current) return null;
    const operation: FileSpaceOperation = {
      token: Symbol(action),
      action,
      generation: lifecycleRef.current.generation,
    };
    operationRef.current = operation;
    nativeDropBlockedRef.current = true;
    setFileDragOver(false);
    setBusyAction(action);
    return operation;
  };

  const canCommitOperation = (operation: FileSpaceOperation) => (
    operationRef.current?.token === operation.token &&
    lifecycleRef.current.mounted &&
    lifecycleRef.current.generation === operation.generation
  );

  const finishOperation = (operation: FileSpaceOperation) => {
    if (operationRef.current?.token !== operation.token) return;
    operationRef.current = null;
    if (
      lifecycleRef.current.mounted &&
      lifecycleRef.current.generation === operation.generation
    ) {
      setBusyAction(null);
      setFileDragOver(false);
    }
  };

  const loadSnapshot = async () => {
    const generation = lifecycleRef.current.generation;
    setLoading(true);
    setError(null);
    try {
      const loaded = await invoke<FileSpaceSnapshot>("get_file_space_snapshot");
      if (
        !lifecycleRef.current.mounted ||
        lifecycleRef.current.generation !== generation
      ) return;
      setSnapshot(loaded);
      setExpandedFolders(new Set(loaded.folders.map((folder) => folder.id)));
      setCurrentFolderId((current) => (
        current && loaded.folders.some((folder) => folder.id === current) ? current : null
      ));
    } catch (loadError) {
      if (
        lifecycleRef.current.mounted &&
        lifecycleRef.current.generation === generation
      ) {
        setError(`${t("fileSpace.errors.load")} ${errorText(loadError)}`);
      }
    } finally {
      if (
        lifecycleRef.current.mounted &&
        lifecycleRef.current.generation === generation
      ) {
        setLoading(false);
      }
    }
  };

  useEffect(() => {
    const generation = lifecycleRef.current.generation + 1;
    lifecycleRef.current = { mounted: true, generation };
    return () => {
      if (lifecycleRef.current.generation === generation) {
        lifecycleRef.current = { mounted: false, generation: generation + 1 };
      }
      if (fileDragReleaseTimerRef.current !== null) {
        window.clearTimeout(fileDragReleaseTimerRef.current);
        fileDragReleaseTimerRef.current = null;
      }
      fileDragStartingRef.current = false;
      fileDragGestureRef.current = null;
      internalFileDragRef.current = null;
      folderDragGestureRef.current = null;
      internalFolderDragRef.current = null;
      if (folderClickReleaseTimerRef.current !== null) {
        window.clearTimeout(folderClickReleaseTimerRef.current);
        folderClickReleaseTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    document.body.classList.toggle("is-file-space-reordering", Boolean(internalFileDrag));
    return () => document.body.classList.remove("is-file-space-reordering");
  }, [internalFileDrag]);

  useEffect(() => {
    document.body.classList.toggle("is-folder-tree-dragging", Boolean(internalFolderDrag));
    return () => document.body.classList.remove("is-folder-tree-dragging");
  }, [internalFolderDrag]);

  useEffect(() => {
    if (!isTauri()) {
      if (import.meta.env.DEV && new URLSearchParams(window.location.search).has("fileSpacePreview")) {
        const fixture = createVisualFixture();
        setSnapshot(fixture);
        setExpandedFolders(new Set(fixture.folders.map((folder) => folder.id)));
        setCurrentFolderId("fixture-research");
      }
      setLoading(false);
      return;
    }
    void loadSnapshot();
    // Initial load belongs to the page lifecycle only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (currentFolderId) {
      window.localStorage.setItem(currentFolderStorageKey, currentFolderId);
    } else {
      window.localStorage.removeItem(currentFolderStorageKey);
    }
  }, [currentFolderId]);

  useEffect(() => {
    const applySavedMarkdownSnapshot = (event: Event) => {
      const next = (event as CustomEvent<FileSpaceSnapshot>).detail;
      if (next) setSnapshot(next);
    };
    window.addEventListener("lumetrace:file-space-snapshot", applySavedMarkdownSnapshot);
    return () => window.removeEventListener("lumetrace:file-space-snapshot", applySavedMarkdownSnapshot);
  }, []);

  useEffect(() => {
    if (!isTauri()) return undefined;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<FileSpaceImportFeedback>("file-space-import-progress", ({ payload }) => {
      if (disposed || payload.requestId !== importRequestRef.current) return;
      setImportFeedback((current) => ({
        requestId: payload.requestId,
        phase: payload.phase,
        processed: payload.processed,
        total: payload.total,
        currentName: payload.currentName,
        error: current?.error,
      }));
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  useEffect(() => {
    if (!importFeedback || !["completed", "cancelled"].includes(importFeedback.phase)) return undefined;
    const timer = window.setTimeout(() => setImportFeedback(null), 3200);
    return () => window.clearTimeout(timer);
  }, [importFeedback]);

  useEffect(() => {
    if (!fileMoveFeedback) return undefined;
    const timer = window.setTimeout(() => setFileMoveFeedback(null), 2600);
    return () => window.clearTimeout(timer);
  }, [fileMoveFeedback]);

  useEffect(() => {
    if (!isCreateFolderOpen) return;
    window.requestAnimationFrame(() => folderNameRef.current?.focus());
  }, [isCreateFolderOpen]);

  useEffect(() => {
    if (!renameFolderId) return;
    window.requestAnimationFrame(() => {
      renameFolderNameRef.current?.focus();
      renameFolderNameRef.current?.select();
    });
  }, [renameFolderId]);

  useEffect(() => {
    if (!renameFileId) return;
    window.requestAnimationFrame(() => {
      renameFileNameRef.current?.focus();
      renameFileNameRef.current?.select();
    });
  }, [renameFileId]);

  useEffect(() => {
    if (!tagFileId) return;
    window.requestAnimationFrame(() => {
      tagDraftRef.current?.focus();
      tagDraftRef.current?.select();
    });
  }, [tagFileId]);

  useEffect(() => {
    if (
      !isCreateFolderOpen &&
      !renameFolderId &&
      !deleteFolderId &&
      !renameFileId &&
      !deleteFileId &&
      !tagFileId &&
      !isFilterMenuOpen &&
      !folderContextMenu &&
      !fileContextMenu
    ) return undefined;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setCreateFolderOpen(false);
      setRenameFolderId(null);
      setDeleteFolderId(null);
      setRenameFileId(null);
      setDeleteFileId(null);
      setTagFileId(null);
      setFilterMenuOpen(false);
      setFolderContextMenu(null);
      setFileContextMenu(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [deleteFileId, deleteFolderId, fileContextMenu, folderContextMenu, isCreateFolderOpen, isFilterMenuOpen, renameFileId, renameFolderId, tagFileId]);

  useEffect(() => {
    if (!isFilterMenuOpen) return undefined;
    const closeFilterMenu = (event: PointerEvent) => {
      if (!filterMenuRef.current?.contains(event.target as Node)) setFilterMenuOpen(false);
    };
    window.addEventListener("pointerdown", closeFilterMenu);
    return () => window.removeEventListener("pointerdown", closeFilterMenu);
  }, [isFilterMenuOpen]);

  useEffect(() => {
    if (!folderContextMenu && !fileContextMenu) return undefined;
    const closeContextMenu = (event: PointerEvent) => {
      if (!contextMenuRef.current?.contains(event.target as Node)) {
        setFolderContextMenu(null);
        setFileContextMenu(null);
      }
    };
    const closeForLayoutChange = () => {
      setFolderContextMenu(null);
      setFileContextMenu(null);
    };
    window.addEventListener("pointerdown", closeContextMenu);
    window.addEventListener("resize", closeForLayoutChange);
    window.addEventListener("scroll", closeForLayoutChange, true);
    return () => {
      window.removeEventListener("pointerdown", closeContextMenu);
      window.removeEventListener("resize", closeForLayoutChange);
      window.removeEventListener("scroll", closeForLayoutChange, true);
    };
  }, [fileContextMenu, folderContextMenu]);

  const foldersByParent = useMemo(() => {
    const groups = new Map<string | null, FileSpaceFolderRecord[]>();
    snapshot.folders.forEach((folder) => {
      const siblings = groups.get(folder.parentId) ?? [];
      siblings.push(folder);
      groups.set(folder.parentId, siblings);
    });
    groups.forEach((folders) => folders.sort((left, right) => (
      left.manualOrder - right.manualOrder
      || left.name.localeCompare(right.name, locale)
      || left.id.localeCompare(right.id)
    )));
    return groups;
  }, [locale, snapshot.folders]);

  const folderCounts = useMemo(() => {
    const counts = new Map<string, number>();
    const countFolder = (folderId: string): number => {
      const directFiles = snapshot.files.filter((file) => file.folderId === folderId).length;
      const childCount = (foldersByParent.get(folderId) ?? [])
        .reduce((total, child) => total + countFolder(child.id), 0);
      const total = directFiles + childCount;
      counts.set(folderId, total);
      return total;
    };
    (foldersByParent.get(null) ?? []).forEach((folder) => countFolder(folder.id));
    return counts;
  }, [foldersByParent, snapshot.files]);

  const folderDisplayDetails = useMemo(() => {
    const filesByFolder = new Map<string, FileSpaceFileRecord[]>();
    snapshot.files.forEach((file) => {
      if (!file.folderId) return;
      const files = filesByFolder.get(file.folderId) ?? [];
      files.push(file);
      filesByFolder.set(file.folderId, files);
    });
    const details = new Map<string, { fileCount: number; folderCount: number; previews: FileSpaceFileRecord[] }>();
    snapshot.folders.forEach((folder) => {
      const directFiles = filesByFolder.get(folder.id) ?? [];
      details.set(folder.id, {
        fileCount: directFiles.length,
        folderCount: (foldersByParent.get(folder.id) ?? []).length,
        previews: directFiles.slice(0, 4),
      });
    });
    return details;
  }, [foldersByParent, snapshot.files, snapshot.folders]);

  const currentFolder = snapshot.folders.find((folder) => folder.id === currentFolderId) ?? null;
  const folderBeingDragged = internalFolderDrag
    ? snapshot.folders.find((folder) => folder.id === internalFolderDrag.folderId) ?? null
    : null;
  const deletedFile = snapshot.files.find((file) => file.id === deleteFileId) ?? null;
  const childFolders = foldersByParent.get(currentFolder?.id ?? null) ?? [];
  const allTags = [...new Set(snapshot.files.flatMap((file) => file.tags))]
    .sort((left, right) => left.localeCompare(right, locale));
  const directFiles = currentFolder
    ? snapshot.files.filter((file) => file.folderId === currentFolder.id)
    : snapshot.files;
  const normalizedQuery = query.trim();
  const activeSearchResult = searchResult?.key === searchKey ? searchResult.ids : null;
  const updatedAfter = timeFilterStart(timeFilter);
  const visibleFolders = childFolders.filter((folder) => (
    !normalizedQuery || (searchScopes.includes("name") && matchesSearch(folder.name, normalizedQuery))
  ));
  const sortedVisibleFiles = directFiles
    .filter((file) => (
      (!normalizedQuery || activeSearchResult?.has(file.id))
      && (typeFilter === "all" || fileCategory(file) === typeFilter)
      && (tagFilter === "all" || file.tags.includes(tagFilter))
      && (updatedAfter === null || file.updatedAt >= updatedAfter)
    ))
    .sort((left, right) => compareFiles(left, right, sortOption, locale));
  const visibleFileById = new Map(sortedVisibleFiles.map((file) => [file.id, file]));
  const visibleFiles = internalFileDrag
    ? internalFileDrag.orderedIds.flatMap((id) => {
        const file = visibleFileById.get(id);
        return file ? [file] : [];
      })
    : sortedVisibleFiles;
  const canReorderVisibleFiles = !normalizedQuery
    && typeFilter === "all"
    && tagFilter === "all"
    && timeFilter === "all"
    && sortedVisibleFiles.length > 1;
  const internallyDraggedFile = internalFileDrag
    ? snapshot.files.find((file) => file.id === internalFileDrag.fileId) ?? null
    : null;
  const visibleFileLayoutKey = visibleFiles
    .map((file) => `${file.id}:${file.name}:${file.updatedAt}`)
    .join("\u0000");

  useLayoutEffect(() => {
    const grid = fileGridRef.current;
    if (!grid) return undefined;

    let animationFrame = 0;
    let disposed = false;
    const cardElements = Array.from(
      grid.querySelectorAll<HTMLElement>(".file-space-file-card"),
    );
    const detailElements = cardElements.flatMap((card) => {
      const details = card.querySelector<HTMLElement>(".file-space-item-copy");
      return details ? [details] : [];
    });
    const imageElements = cardElements.flatMap((card) => {
      const image = card.querySelector<HTMLImageElement>(".file-space-file-art img");
      return image ? [image] : [];
    });
    const measure = () => {
      animationFrame = 0;
      if (disposed) return;
      const containerWidth = grid.getBoundingClientRect().width;
      if (containerWidth <= 0) return;

      const items = cardElements.flatMap((card) => {
        const id = card.dataset.fileId;
        const fallbackRatio = Number(card.dataset.previewAspectRatio) || 1;
        const image = card.querySelector<HTMLImageElement>(".file-space-file-art img");
        const imageRatio = image?.naturalWidth && image.naturalHeight
          ? image.naturalWidth / image.naturalHeight
          : fallbackRatio;
        return id ? [{ id, aspectRatio: imageRatio }] : [];
      });
      const detailsHeight = Math.ceil(detailElements.reduce((maximum, details) => (
        Math.max(maximum, details.getBoundingClientRect().height)
      ), 0));
      const nextLayout = calculateJustifiedFileLayout({
        containerWidth,
        targetPreviewHeight: previewSize,
        horizontalGap: fileLayoutHorizontalGap,
        verticalGap: fileLayoutVerticalGap,
        previewDetailsGap: filePreviewDetailsGap,
        detailsHeight,
        items,
      });
      setFileJustifiedLayout((current) => (
        justifiedFileLayoutsEqual(current, nextLayout) ? current : nextLayout
      ));
    };
    const scheduleMeasure = () => {
      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      animationFrame = window.requestAnimationFrame(measure);
    };
    const resizeObserver = new ResizeObserver(scheduleMeasure);

    resizeObserver.observe(grid);
    detailElements.forEach((details) => resizeObserver.observe(details));
    imageElements.forEach((image) => image.addEventListener("load", scheduleMeasure));
    measure();

    return () => {
      disposed = true;
      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      imageElements.forEach((image) => image.removeEventListener("load", scheduleMeasure));
      resizeObserver.disconnect();
    };
  }, [previewSize, visibleFileLayoutKey]);

  const openTimeline = async (file: FileSpaceFileRecord) => {
    if (file.versionCount < 1 || !isTauri()) return;
    setFileContextMenu(null);
    setTimelineBusy(true);
    setError(null);
    try {
      const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: file.id });
      setTimelineFile(file);
      setTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
      setVersionPreview(null);
    } catch (timelineError) {
      setError(errorText(timelineError));
    } finally {
      setTimelineBusy(false);
    }
  };

  const selectTimelineVersion = async (version: TaskFileVersionRecord) => {
    setSelectedVersionId(version.id);
    if (!supportsTextPreview(version)) {
      setVersionPreview(null);
      return;
    }
    setTimelineBusy(true);
    try {
      const bytes = await invoke<number[]>("read_task_file_version", {
        fileId: timeline?.fileId,
        versionId: version.id,
      });
      setVersionPreview(new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes)));
    } catch {
      setVersionPreview(null);
    } finally {
      setTimelineBusy(false);
    }
  };

  const setCurrentTimelineVersion = async (versionId: string) => {
    if (!timeline) return;
    setTimelineBusy(true);
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("set_current_task_file_version", {
        fileId: timeline.fileId,
        versionId,
      });
      const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: timeline.fileId });
      setSnapshot(updated);
      setTimeline(loaded);
      setTimelineFile(updated.files.find((file) => file.id === timeline.fileId) ?? timelineFile);
    } catch (versionError) {
      setError(errorText(versionError));
    } finally {
      setTimelineBusy(false);
    }
  };

  const folderIsActuallyEmpty = childFolders.length === 0 && directFiles.length === 0;
  const showEmptyDropZone = folderIsActuallyEmpty
    && !normalizedQuery
    && typeFilter === "all"
    && tagFilter === "all"
    && timeFilter === "all";
  const workspaceTitle = currentFolder?.name ?? snapshot.rootName ?? t("fileSpace.title");
  const hasActiveFilters = typeFilter !== "all"
    || tagFilter !== "all"
    || timeFilter !== "all"
    || sortOption !== "updatedDesc"
    || searchScopes.length !== 3;
  const chooseStorageRoot = async (mode: RootSetupMode) => {
    if (!isTauri()) {
      setError(`${t("fileSpace.errors.configure")} Tauri desktop runtime is required.`);
      return;
    }
    const operation = beginOperation("configure");
    if (!operation) return;
    setRootSetupMode(mode);
    setSetupCancelled(false);
    setError(null);
    try {
      await waitForCommittedPaint();
      if (!canCommitOperation(operation)) return;
      const selected = await open({ directory: true, multiple: false });
      if (!canCommitOperation(operation)) return;
      if (!selected) {
        setSetupCancelled(true);
        return;
      }
      const command = mode === "import"
        ? "import_existing_file_space_root"
        : "configure_file_space_root";
      const configured = await invoke<FileSpaceSnapshot>(command, {
        path: selected,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(configured);
      setCurrentFolderId(null);
      setExpandedFolders(new Set(configured.folders.map((folder) => folder.id)));
    } catch (configureError) {
      if (canCommitOperation(operation)) {
        const errorKey = mode === "import" ? "fileSpace.errors.initializeImport" : "fileSpace.errors.configure";
        setError(`${t(errorKey)} ${errorText(configureError)}`);
      }
    } finally {
      finishOperation(operation);
      setRootSetupMode(null);
    }
  };

  const createFolder = async () => {
    if (!folderName.trim()) return;
    const operation = beginOperation("create");
    if (!operation) return;
    setError(null);
    try {
      const folder = await invoke<FileSpaceFolderRecord>("create_file_space_folder", {
        parentId: createParentId,
        name: folderName.trim(),
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot((current) => ({ ...current, folders: [...current.folders, folder] }));
      if (createParentId) {
        setExpandedFolders((current) => new Set(current).add(createParentId));
      }
      setFolderName("");
      setCreateFolderOpen(false);
      setCreateParentId(null);
    } catch (createError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.createFolder")} ${errorText(createError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const openCreateFolder = (parentId: string | null) => {
    if (operationRef.current) return;
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setCreateParentId(parentId);
    setFolderName("");
    setCreateFolderOpen(true);
  };

  const importFiles = async (
    paths?: string[],
    destinationFolderId: string | null = currentFolder?.id ?? null,
  ) => {
    if (!isTauri()) {
      setError(`${t("fileSpace.errors.import")} Tauri desktop runtime is required.`);
      return;
    }
    const operation = beginOperation("import");
    if (!operation) return;
    let requestId: string | null = null;
    setError(null);
    try {
      let selectedPaths = paths;
      if (!selectedPaths) {
        await waitForCommittedPaint();
        if (!canCommitOperation(operation)) return;
        const selected = await open({ directory: false, multiple: true });
        if (!canCommitOperation(operation)) return;
        if (!selected) return;
        selectedPaths = Array.isArray(selected) ? selected : [selected];
      }
      if (selectedPaths.length === 0) return;
      requestId = createImportRequestId();
      importRequestRef.current = requestId;
      setImportFeedback({ requestId, phase: "scanning", processed: 0, total: 0, currentName: null });
      const imported = await invoke<FileSpaceSnapshot>("import_file_space_files", {
        requestId,
        folderId: destinationFolderId,
        paths: selectedPaths,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(imported);
      setExpandedFolders(new Set(imported.folders.map((folder) => folder.id)));
      setImportFeedback((current) => current?.requestId === requestId
        ? { ...current, phase: "completed", processed: current.total || current.processed }
        : current);
    } catch (importError) {
      if (canCommitOperation(operation)) {
        const message = errorText(importError);
        if (message.includes("IMPORT_CANCELLED:")) {
          setImportFeedback((current) => current && current.requestId === requestId
            ? { ...current, phase: "cancelled", error: undefined }
            : current);
        } else {
          setImportFeedback((current) => current && current.requestId === requestId
            ? { ...current, phase: "failed", error: message }
            : current);
          setError(`${t("fileSpace.errors.import")} ${message}`);
        }
      }
    } finally {
      if (importRequestRef.current === requestId) importRequestRef.current = null;
      finishOperation(operation);
    }
  };

  const importBrowserDroppedFiles = async (
    droppedFiles: BrowserDroppedFile[],
    destinationFolderId: string | null,
  ) => {
    if (!isTauri()) {
      setError(`${t("fileSpace.errors.import")} Tauri desktop runtime is required.`);
      return;
    }
    const operation = beginOperation("import");
    if (!operation) return;
    const requestId = createImportRequestId();
    importRequestRef.current = requestId;
    setError(null);
    setImportFeedback({
      requestId,
      phase: "scanning",
      processed: 0,
      total: droppedFiles.length,
      currentName: null,
    });
    try {
      const files: FileSpaceDroppedFilePayload[] = [];
      for (const dropped of droppedFiles) {
        if (!canCommitOperation(operation)) return;
        setImportFeedback((current) => current?.requestId === requestId
          ? { ...current, currentName: dropped.relativePath }
          : current);
        const buffer = new Uint8Array(await dropped.file.arrayBuffer());
        files.push({
          relativePath: dropped.relativePath,
          bytes: Array.from(buffer),
        });
      }
      if (!canCommitOperation(operation)) return;
      const imported = await invoke<FileSpaceSnapshot>("import_file_space_dropped_files", {
        requestId,
        folderId: destinationFolderId,
        files,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(imported);
      setExpandedFolders(new Set(imported.folders.map((folder) => folder.id)));
      setImportFeedback((current) => current?.requestId === requestId
        ? { ...current, phase: "completed", processed: current.total || current.processed, currentName: null }
        : current);
    } catch (importError) {
      if (canCommitOperation(operation)) {
        const message = errorText(importError);
        if (message.includes("IMPORT_CANCELLED:")) {
          setImportFeedback((current) => current?.requestId === requestId
            ? { ...current, phase: "cancelled", error: undefined, currentName: null }
            : current);
        } else {
          setImportFeedback((current) => current?.requestId === requestId
            ? { ...current, phase: "failed", error: message, currentName: null }
            : current);
          setError(`${t("fileSpace.errors.import")} ${message}`);
        }
      }
    } finally {
      if (importRequestRef.current === requestId) importRequestRef.current = null;
      finishOperation(operation);
    }
  };

  const cancelImport = async () => {
    const requestId = importRequestRef.current;
    if (!requestId) return;
    setImportFeedback((current) => current?.requestId === requestId ? { ...current, phase: "cancelling" } : current);
    try {
      await invoke("cancel_file_space_import", { requestId });
    } catch (cancelError) {
      setError(`${t("fileSpace.errors.cancelImport")} ${errorText(cancelError)}`);
    }
  };

  const nativeDropBlocked = Boolean(
    internalFileDrag ||
    internalFolderDrag ||
    busyAction ||
    operationRef.current ||
    folderContextMenu ||
    fileContextMenu ||
    isFilterMenuOpen ||
    isCreateFolderOpen ||
    renameFolderId ||
    deleteFolderId ||
    renameFileId ||
    deleteFileId ||
    tagFileId ||
    createDialogPresence.mounted ||
    renameDialogPresence.mounted ||
    deleteDialogPresence.mounted ||
    renameFileDialogPresence.mounted ||
    deleteFileDialogPresence.mounted
    || tagDialogPresence.mounted
  );

  useEffect(() => {
    nativeDropBlockedRef.current = nativeDropBlocked;
    if (nativeDropBlocked) {
      externalDragDepthRef.current = 0;
      setFileDragOver(false);
    }
  }, [nativeDropBlocked]);

  const suppressOutgoingSelfDrop = (event: React.DragEvent<HTMLDivElement>) => {
    if (!fileDragStartingRef.current) return false;
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "none";
    externalDragDepthRef.current = 0;
    setFileDragOver(false);
    return true;
  };

  const handleExternalDragEnter = (event: React.DragEvent<HTMLDivElement>) => {
    if (suppressOutgoingSelfDrop(event)) return;
    if (nativeDropBlockedRef.current || operationRef.current) return;
    if (!supportsExternalFileDrop(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    externalDragDepthRef.current += 1;
    setFileDragOver(true);
  };

  const handleExternalDragOver = (event: React.DragEvent<HTMLDivElement>) => {
    if (suppressOutgoingSelfDrop(event)) return;
    if (nativeDropBlockedRef.current || operationRef.current) return;
    if (!supportsExternalFileDrop(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "copy";
    setFileDragOver(true);
  };

  const handleExternalDragLeave = (event: React.DragEvent<HTMLDivElement>) => {
    if (suppressOutgoingSelfDrop(event)) return;
    if (!supportsExternalFileDrop(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    externalDragDepthRef.current = Math.max(0, externalDragDepthRef.current - 1);
    if (externalDragDepthRef.current === 0) setFileDragOver(false);
  };

  const handleExternalDrop = (event: React.DragEvent<HTMLDivElement>) => {
    if (suppressOutgoingSelfDrop(event)) return;
    if (nativeDropBlockedRef.current || operationRef.current) return;
    if (!supportsExternalFileDrop(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    externalDragDepthRef.current = 0;
    setFileDragOver(false);
    const dataTransfer = event.dataTransfer;
    const localPaths = localPathsFromUriList(dataTransfer);
    const destinationFolderId = currentFolder?.id ?? null;
    void (async () => {
      try {
        // Prefer real local paths when the source exposes them. Besides avoiding an
        // unnecessary in-memory copy, this preserves empty directories from Finder.
        if (localPaths.length > 0) {
          await importFiles(localPaths, destinationFolderId);
          return;
        }
        const droppedFiles = await collectBrowserDroppedFiles(dataTransfer);
        if (droppedFiles.length > 0) {
          await importBrowserDroppedFiles(droppedFiles, destinationFolderId);
          return;
        }
        setError(t("fileSpace.errors.unsupportedDrop"));
      } catch (dropError) {
        setError(`${t("fileSpace.errors.dragDrop")} ${errorText(dropError)}`);
      }
    })();
  };

  const openFolderContextMenu = (event: React.MouseEvent, folderId: string) => {
    event.preventDefault();
    event.stopPropagation();
    if (operationRef.current) return;
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    const menuWidth = 190;
    const menuHeight = 128;
    setFolderContextMenu({
      folderId,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
    });
  };

  const openFileContextMenu = (event: React.MouseEvent, fileId: string) => {
    event.preventDefault();
    event.stopPropagation();
    if (operationRef.current) return;
    nativeDropBlockedRef.current = true;
    const menuWidth = 218;
    const file = snapshot.files.find((item) => item.id === fileId);
    const menuHeight = file?.versionCount ? 280 : 236;
    setFolderContextMenu(null);
    setFileContextMenu({
      fileId,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
    });
  };

  const toggleSearchScope = (scope: SearchScope) => {
    setSearchScopes((current) => {
      if (current.includes(scope)) {
        return current.length === 1 ? current : current.filter((item) => item !== scope);
      }
      return [...current, scope];
    });
  };

  const resetFilters = () => {
    setTypeFilter("all");
    setTagFilter("all");
    setTimeFilter("all");
    updateSortOption("updatedDesc");
    setSearchScopes(["name", "content", "tag"]);
  };

  const openTagDialog = (fileId: string) => {
    if (operationRef.current) return;
    const file = snapshot.files.find((item) => item.id === fileId);
    if (!file) return;
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    setTagDraft(file.tags.join(", "));
    setTagFileId(fileId);
  };

  const saveFileTags = async () => {
    if (!tagFileId) return;
    const operation = beginOperation("fileAction");
    if (!operation) return;
    setError(null);
    try {
      const tags = tagDraft
        .split(/[,，\n]/)
        .map((tag) => tag.trim())
        .filter(Boolean);
      const updated = await invoke<FileSpaceSnapshot>("set_file_space_file_tags", {
        fileId: tagFileId,
        tags,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setTagFileId(null);
    } catch (tagError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.tags")} ${errorText(tagError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const runFileCommand = async (
    command: "reveal_file_space_file" | "copy_file_space_file" | "copy_file_space_file_path",
    fileId: string,
    errorKey: "revealFile" | "copyFile" | "copyPath",
  ) => {
    const operation = beginOperation("fileAction");
    if (!operation) return;
    setFileContextMenu(null);
    setError(null);
    try {
      await invoke(command, { fileId });
    } catch (commandError) {
      if (canCommitOperation(operation)) {
        setError(`${t(`fileSpace.errors.${errorKey}`)} ${errorText(commandError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const startFileDragOut = async (target: HTMLButtonElement, fileId: string) => {
    if (!isTauri() || operationRef.current || fileDragStartingRef.current) return;
    if (fileDragReleaseTimerRef.current !== null) {
      window.clearTimeout(fileDragReleaseTimerRef.current);
      fileDragReleaseTimerRef.current = null;
    }
    fileDragStartingRef.current = true;
    setFileContextMenu(null);
    setError(null);
    const previewBytes = await fileDragPreviewBytes(target);
    if (!lifecycleRef.current.mounted || !fileDragStartingRef.current) return;
    void invoke("start_file_space_drag_out", { fileId, previewBytes })
      .catch((dragError) => {
        setError(`${t("fileSpace.errors.dragOut")} ${errorText(dragError)}`);
      })
      .finally(() => {
        // macOS ends the native session before WebKit dispatches its final drop.
        // Keep the self-drop guard alive through that later browser event.
        fileDragReleaseTimerRef.current = window.setTimeout(() => {
          fileDragReleaseTimerRef.current = null;
          fileDragStartingRef.current = false;
          externalDragDepthRef.current = 0;
          if (lifecycleRef.current.mounted) setFileDragOver(false);
        }, 750);
      });
  };

  const cancelInternalFileDrag = () => {
    updateInternalFileDrag(null);
  };

  const persistFileOrder = async (orderedIds: string[]) => {
    const previousSnapshot = snapshot;
    if (!isTauri()) {
      updateSortOption("manual");
      setSnapshot(optimisticManualOrder(previousSnapshot, orderedIds));
      return;
    }
    const operation = beginOperation("reorder");
    if (!operation) return;
    setError(null);
    updateSortOption("manual");
    setSnapshot(optimisticManualOrder(previousSnapshot, orderedIds));
    try {
      const updated = await invoke<FileSpaceSnapshot>("reorder_file_space_files", {
        orderedFileIds: orderedIds,
      });
      if (canCommitOperation(operation)) setSnapshot(updated);
    } catch (reorderError) {
      if (canCommitOperation(operation)) {
        setSnapshot(previousSnapshot);
        setError(`${t("fileSpace.errors.reorder")} ${errorText(reorderError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const promoteToNativeFileDrag = (gesture: FileDragGesture) => {
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
    if (!isTauri()) return;
    void startFileDragOut(gesture.target, gesture.fileId);
  };

  const resolveFileFolderDropTarget = (
    clientX: number,
    clientY: number,
    fileId: string,
  ): FileFolderDropTarget | null => {
    const element = document.elementFromPoint(clientX, clientY);
    if (!(element instanceof Element)) return null;
    const sourceFolderId = snapshot.files.find((file) => file.id === fileId)?.folderId ?? null;
    const row = element.closest<HTMLElement>("[data-folder-tree-id]");
    const targetId = row?.dataset.folderTreeId;
    if (targetId && snapshot.folders.some((folder) => folder.id === targetId)) {
      return {
        folderId: targetId,
        targetId,
        kind: "folder",
        isCurrent: sourceFolderId === targetId,
      };
    }
    if (element.closest<HTMLElement>("[data-file-root-drop]")) {
      return {
        folderId: null,
        targetId: null,
        kind: "root",
        isCurrent: sourceFolderId === null,
      };
    }
    return null;
  };

  const beginFileDragGesture = (
    event: React.PointerEvent<HTMLButtonElement>,
    fileId: string,
  ) => {
    if (event.button !== 0 || operationRef.current) return;
    const card = event.currentTarget.closest<HTMLElement>(".file-space-file-card");
    const rect = card?.getBoundingClientRect() ?? event.currentTarget.getBoundingClientRect();
    fileDragGestureRef.current = {
      fileId,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      offsetX: event.clientX - rect.left,
      offsetY: event.clientY - rect.top,
      cardWidth: rect.width,
      cardHeight: rect.height,
      target: event.currentTarget,
      orderedIds: sortedVisibleFiles.map((file) => file.id),
      phase: "pending",
    };
  };

  const continueFileDragGesture = (event: PointerEvent) => {
    const gesture = fileDragGestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    const fileId = gesture.fileId;
    if ((event.buttons & 1) === 0) {
      fileDragGestureRef.current = null;
      cancelInternalFileDrag();
      return;
    }
    if (Math.hypot(event.clientX - gesture.startX, event.clientY - gesture.startY) < fileDragThreshold) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (pointerReachedWindowEdge(event.clientX, event.clientY)) {
      promoteToNativeFileDrag(gesture);
      return;
    }

    const current = internalFileDragRef.current;
    const folderTarget = resolveFileFolderDropTarget(event.clientX, event.clientY, fileId);
    if (gesture.phase === "pending" || !current) {
      gesture.phase = "reordering";
      const started: InternalFileDrag = {
        fileId,
        pointerId: event.pointerId,
        pointerX: event.clientX,
        pointerY: event.clientY,
        offsetX: gesture.offsetX,
        offsetY: gesture.offsetY,
        cardWidth: gesture.cardWidth,
        cardHeight: gesture.cardHeight,
        originalIds: gesture.orderedIds,
        orderedIds: gesture.orderedIds,
        folderTarget,
      };
      updateInternalFileDrag(started);
      return;
    }

    const grid = fileGridRef.current;
    const gridRect = grid?.getBoundingClientRect();
    const pointerInGrid = Boolean(
      gridRect
      && event.clientX >= gridRect.left
      && event.clientX <= gridRect.right
      && event.clientY >= gridRect.top
      && event.clientY <= gridRect.bottom,
    );
    const orderedIds = folderTarget
      ? current.originalIds
      : grid && pointerInGrid && canReorderVisibleFiles
      ? reorderedIdsAtPointer(
          grid,
          current.orderedIds,
          fileId,
          event.clientX,
          event.clientY,
          current.offsetX,
          current.offsetY,
          current.cardWidth,
          current.cardHeight,
        )
      : current.orderedIds;
    updateInternalFileDrag({
      ...current,
      pointerX: event.clientX,
      pointerY: event.clientY,
      orderedIds,
      folderTarget,
    });
  };

  const moveFile = async (fileId: string, folderId: string | null) => {
    const sourceFile = snapshot.files.find((file) => file.id === fileId);
    if (!sourceFile) return;
    const destinationName = folderId
      ? snapshot.folders.find((folder) => folder.id === folderId)?.name
      : t("fileSpace.moveFeedback.root");
    if (!destinationName) return;
    setFileMoveFeedback(null);
    if (sourceFile.folderId === folderId) {
      setFileMoveFeedback({
        fileName: sourceFile.name,
        destinationName,
        status: "unchanged",
      });
      return;
    }
    const operation = beginOperation("moveFile");
    if (!operation) return;
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("move_file_space_file", {
        fileId,
        folderId,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setImportFeedback(null);
      setFileMoveFeedback({ fileName: sourceFile.name, destinationName, status: "moved" });
      if (folderId) {
        setExpandedFolders((current) => new Set(current).add(folderId));
      }
    } catch (moveError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.moveFile")} ${errorText(moveError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const finishFileDragGesture = (pointerId: number, clientX: number, clientY: number) => {
    const gesture = fileDragGestureRef.current;
    if (!gesture || gesture.pointerId !== pointerId) return;
    const drag = internalFileDragRef.current;
    const folderTarget = resolveFileFolderDropTarget(clientX, clientY, gesture.fileId);
    const gridRect = fileGridRef.current?.getBoundingClientRect();
    const droppedInGrid = Boolean(
      gridRect
      && clientX >= gridRect.left
      && clientX <= gridRect.right
      && clientY >= gridRect.top
      && clientY <= gridRect.bottom,
    );
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
    if (drag && folderTarget) {
      void moveFile(drag.fileId, folderTarget.folderId);
      return;
    }
    if (
      drag
      && droppedInGrid
      && !sameOrder(drag.originalIds, drag.orderedIds)
    ) {
      void persistFileOrder(drag.orderedIds);
    }
  };

  const cancelFileDragGesture = (pointerId?: number) => {
    const gesture = fileDragGestureRef.current;
    if (!gesture || (pointerId !== undefined && gesture.pointerId !== pointerId)) return;
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
  };

  useEffect(() => {
    const trackGesture = (event: PointerEvent) => continueFileDragGesture(event);
    const finishGesture = (event: PointerEvent) => (
      finishFileDragGesture(event.pointerId, event.clientX, event.clientY)
    );
    const cancelGesture = (event: PointerEvent) => cancelFileDragGesture(event.pointerId);
    const cancelOnBlur = () => cancelFileDragGesture();
    window.addEventListener("pointermove", trackGesture, true);
    window.addEventListener("pointerup", finishGesture, true);
    window.addEventListener("pointercancel", cancelGesture, true);
    window.addEventListener("blur", cancelOnBlur);
    return () => {
      window.removeEventListener("pointermove", trackGesture, true);
      window.removeEventListener("pointerup", finishGesture, true);
      window.removeEventListener("pointercancel", cancelGesture, true);
      window.removeEventListener("blur", cancelOnBlur);
    };
  });

  useEffect(() => {
    const promoteWhenPointerLeavesWindow = (event: PointerEvent) => {
      const gesture = fileDragGestureRef.current;
      if (
        !gesture
        || gesture.pointerId !== event.pointerId
        || event.relatedTarget !== null
        || (event.buttons & 1) === 0
        || Math.hypot(event.clientX - gesture.startX, event.clientY - gesture.startY) < fileDragThreshold
      ) return;
      event.preventDefault();
      promoteToNativeFileDrag(gesture);
    };
    window.addEventListener("pointerout", promoteWhenPointerLeavesWindow, true);
    return () => window.removeEventListener("pointerout", promoteWhenPointerLeavesWindow, true);
  });

  useEffect(() => {
    if (!internalFileDrag) return undefined;
    const cancelWithEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      fileDragGestureRef.current = null;
      cancelInternalFileDrag();
    };
    window.addEventListener("keydown", cancelWithEscape);
    return () => window.removeEventListener("keydown", cancelWithEscape);
  }, [internalFileDrag]);

  const openRenameFile = (fileId: string) => {
    if (operationRef.current) return;
    const file = snapshot.files.find((item) => item.id === fileId);
    if (!file) return;
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    setRenameFileName(file.name);
    setRenameFileId(fileId);
  };

  const renameFile = async () => {
    if (!renameFileId || !renameFileName.trim()) return;
    const operation = beginOperation("rename");
    if (!operation) return;
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("rename_file_space_file", {
        fileId: renameFileId,
        name: renameFileName.trim(),
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setRenameFileId(null);
    } catch (renameError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.renameFile")} ${errorText(renameError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const requestDeleteFile = (fileId: string) => {
    if (operationRef.current) return;
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    setDeleteFileId(fileId);
  };

  const deleteFile = async () => {
    if (!deleteFileId) return;
    const operation = beginOperation("delete");
    if (!operation) return;
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("delete_file_space_file", {
        fileId: deleteFileId,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setDeleteFileId(null);
    } catch (deleteError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.deleteFile")} ${errorText(deleteError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const openRenameFolder = (folderId: string) => {
    if (operationRef.current) return;
    const folder = snapshot.folders.find((item) => item.id === folderId);
    if (!folder) return;
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setRenameFolderName(folder.name);
    setRenameFolderId(folderId);
  };

  const renameFolder = async () => {
    if (!renameFolderId || !renameFolderName.trim()) return;
    const operation = beginOperation("rename");
    if (!operation) return;
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("rename_file_space_folder", {
        folderId: renameFolderId,
        name: renameFolderName.trim(),
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setRenameFolderId(null);
    } catch (renameError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.renameFolder")} ${errorText(renameError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const requestDeleteFolder = (folderId: string) => {
    if (operationRef.current) return;
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setDeleteFolderId(folderId);
  };

  const deleteFolder = async () => {
    if (!deleteFolderId) return;
    const operation = beginOperation("delete");
    if (!operation) return;
    const descendantIds = new Set<string>();
    const collectDescendants = (folderId: string) => {
      descendantIds.add(folderId);
      (foldersByParent.get(folderId) ?? []).forEach((folder) => collectDescendants(folder.id));
    };
    collectDescendants(deleteFolderId);
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("delete_file_space_folder", {
        folderId: deleteFolderId,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setExpandedFolders((current) => new Set([...current].filter((id) => !descendantIds.has(id))));
      if (currentFolderId && descendantIds.has(currentFolderId)) setCurrentFolderId(null);
      setDeleteFolderId(null);
    } catch (deleteError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.deleteFolder")} ${errorText(deleteError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const releaseFolderClickSuppression = () => {
    if (folderClickReleaseTimerRef.current !== null) {
      window.clearTimeout(folderClickReleaseTimerRef.current);
    }
    folderClickReleaseTimerRef.current = window.setTimeout(() => {
      folderClickReleaseTimerRef.current = null;
      suppressFolderClickRef.current = false;
    }, 0);
  };

  const cancelFolderDragGesture = (pointerId?: number) => {
    const gesture = folderDragGestureRef.current;
    if (!gesture || (pointerId !== undefined && gesture.pointerId !== pointerId)) return;
    folderDragGestureRef.current = null;
    updateInternalFolderDrag(null);
    if (gesture.phase === "dragging") releaseFolderClickSuppression();
  };

  const folderDescendantIds = (folderId: string) => {
    const descendants = new Set<string>([folderId]);
    const collect = (parentId: string) => {
      (foldersByParent.get(parentId) ?? []).forEach((folder) => {
        if (descendants.has(folder.id)) return;
        descendants.add(folder.id);
        collect(folder.id);
      });
    };
    collect(folderId);
    return descendants;
  };

  const resolveFolderDropTarget = (
    gesture: FolderDragGesture,
    clientX: number,
    clientY: number,
  ): FolderDropTarget | null => {
    const element = document.elementFromPoint(clientX, clientY);
    if (!(element instanceof Element)) return null;
    const draggedFolder = snapshot.folders.find((folder) => folder.id === gesture.folderId);
    if (!draggedFolder) return null;
    const descendants = folderDescendantIds(gesture.folderId);
    const row = element.closest<HTMLElement>("[data-folder-tree-id]");
    if (row) {
      const targetId = row.dataset.folderTreeId;
      const targetFolder = snapshot.folders.find((folder) => folder.id === targetId);
      if (!targetId || !targetFolder || targetId === gesture.folderId) return null;
      const rect = row.getBoundingClientRect();
      const ratio = Math.max(0, Math.min(1, (clientY - rect.top) / Math.max(rect.height, 1)));
      const mode: Exclude<FolderDropMode, "root"> = ratio < 0.27
        ? "before"
        : ratio > 0.73
          ? "after"
          : "inside";
      const parentId = mode === "inside" ? targetId : targetFolder.parentId;
      if (parentId === gesture.folderId || (parentId && descendants.has(parentId))) return null;
      const siblings = (foldersByParent.get(parentId) ?? [])
        .map((folder) => folder.id)
        .filter((id) => id !== gesture.folderId);
      if (mode === "inside") {
        return {
          mode,
          targetId,
          parentId,
          orderedSiblingIds: [...siblings, gesture.folderId],
        };
      }
      const targetIndex = siblings.indexOf(targetId);
      if (targetIndex < 0) return null;
      const insertionIndex = mode === "before" ? targetIndex : targetIndex + 1;
      const orderedSiblingIds = [...siblings];
      orderedSiblingIds.splice(insertionIndex, 0, gesture.folderId);
      return { mode, targetId, parentId, orderedSiblingIds };
    }

    const rootDrop = element.closest<HTMLElement>("[data-folder-root-drop]");
    const tree = folderTreeRef.current;
    if (!rootDrop && (!tree || !tree.contains(element))) return null;
    const orderedSiblingIds = (foldersByParent.get(null) ?? [])
      .map((folder) => folder.id)
      .filter((id) => id !== gesture.folderId);
    orderedSiblingIds.push(gesture.folderId);
    return {
      mode: "root",
      targetId: null,
      parentId: null,
      orderedSiblingIds,
    };
  };

  const moveFolder = async (drag: InternalFolderDrag, target: FolderDropTarget) => {
    const folder = snapshot.folders.find((item) => item.id === drag.folderId);
    if (!folder) return;
    const currentSiblingIds = (foldersByParent.get(folder.parentId) ?? []).map((item) => item.id);
    if (folder.parentId === target.parentId && sameOrder(currentSiblingIds, target.orderedSiblingIds)) {
      return;
    }
    const operation = beginOperation("moveFolder");
    if (!operation) return;
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("move_file_space_folder", {
        folderId: drag.folderId,
        parentId: target.parentId,
        orderedSiblingIds: target.orderedSiblingIds,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      if (target.parentId) {
        setExpandedFolders((current) => new Set(current).add(target.parentId!));
      }
    } catch (moveError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.moveFolder")} ${errorText(moveError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const beginFolderDragGesture = (
    event: React.PointerEvent<HTMLButtonElement>,
    folderId: string,
  ) => {
    if (event.button !== 0 || operationRef.current || internalFileDragRef.current) return;
    const row = event.currentTarget.closest<HTMLElement>(".file-space-tree-row");
    const rect = row?.getBoundingClientRect() ?? event.currentTarget.getBoundingClientRect();
    folderDragGestureRef.current = {
      folderId,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      offsetX: event.clientX - rect.left,
      offsetY: event.clientY - rect.top,
      rowWidth: rect.width,
      rowHeight: rect.height,
      phase: "pending",
    };
  };

  const continueFolderDragGesture = (event: PointerEvent) => {
    const gesture = folderDragGestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    if ((event.buttons & 1) === 0) {
      cancelFolderDragGesture(event.pointerId);
      return;
    }
    if (Math.hypot(event.clientX - gesture.startX, event.clientY - gesture.startY) < folderDragThreshold) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (gesture.phase === "pending") {
      gesture.phase = "dragging";
      suppressFolderClickRef.current = true;
      setFolderContextMenu(null);
    }
    const tree = folderTreeRef.current;
    if (tree) {
      const rect = tree.getBoundingClientRect();
      if (event.clientY < rect.top + 26) tree.scrollTop -= 9;
      else if (event.clientY > rect.bottom - 26) tree.scrollTop += 9;
    }
    updateInternalFolderDrag({
      folderId: gesture.folderId,
      pointerId: gesture.pointerId,
      pointerX: event.clientX,
      pointerY: event.clientY,
      offsetX: gesture.offsetX,
      offsetY: gesture.offsetY,
      rowWidth: gesture.rowWidth,
      rowHeight: gesture.rowHeight,
      target: resolveFolderDropTarget(gesture, event.clientX, event.clientY),
    });
  };

  const finishFolderDragGesture = (event: PointerEvent) => {
    const gesture = folderDragGestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    const drag = internalFolderDragRef.current;
    folderDragGestureRef.current = null;
    updateInternalFolderDrag(null);
    if (gesture.phase === "dragging") releaseFolderClickSuppression();
    if (drag?.target) void moveFolder(drag, drag.target);
  };

  useEffect(() => {
    const track = (event: PointerEvent) => continueFolderDragGesture(event);
    const finish = (event: PointerEvent) => finishFolderDragGesture(event);
    const cancel = (event: PointerEvent) => cancelFolderDragGesture(event.pointerId);
    const cancelOnBlur = () => cancelFolderDragGesture();
    const cancelOnLeave = (event: PointerEvent) => {
      if (event.relatedTarget === null) cancelFolderDragGesture(event.pointerId);
    };
    const cancelWithEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") cancelFolderDragGesture();
    };
    window.addEventListener("pointermove", track, true);
    window.addEventListener("pointerup", finish, true);
    window.addEventListener("pointercancel", cancel, true);
    window.addEventListener("pointerout", cancelOnLeave, true);
    window.addEventListener("blur", cancelOnBlur);
    window.addEventListener("keydown", cancelWithEscape);
    return () => {
      window.removeEventListener("pointermove", track, true);
      window.removeEventListener("pointerup", finish, true);
      window.removeEventListener("pointercancel", cancel, true);
      window.removeEventListener("pointerout", cancelOnLeave, true);
      window.removeEventListener("blur", cancelOnBlur);
      window.removeEventListener("keydown", cancelWithEscape);
    };
  });

  const chooseFolder = (folderId: string | null) => {
    setCurrentFolderId(folderId);
    setQuery("");
  };

  const toggleExpanded = (folderId: string) => {
    setExpandedFolders((current) => {
      const next = new Set(current);
      if (next.has(folderId)) next.delete(folderId);
      else next.add(folderId);
      return next;
    });
  };

  const renderTree = (parentId: string | null, depth = 0): React.ReactNode => (
    (foldersByParent.get(parentId) ?? []).map((folder) => {
      const hasChildren = (foldersByParent.get(folder.id) ?? []).length > 0;
      const expanded = expandedFolders.has(folder.id);
      const active = currentFolder?.id === folder.id;
      const dropMode = internalFolderDrag?.target?.targetId === folder.id
        ? internalFolderDrag.target.mode
        : null;
      const fileDropTarget = internalFileDrag?.folderTarget?.targetId === folder.id
        ? internalFileDrag.folderTarget
        : null;
      return (
        <div className="file-space-tree-branch" key={folder.id}>
          <div
            className={`file-space-tree-row${active ? " is-active" : ""}${internalFolderDrag?.folderId === folder.id ? " is-drag-placeholder" : ""}${dropMode ? ` is-drop-${dropMode}` : ""}${fileDropTarget ? (fileDropTarget.isCurrent ? " is-file-drop-current" : " is-file-drop-target") : ""}`}
            data-folder-tree-id={folder.id}
            style={{ paddingInlineStart: `${depth * 17}px` }}
            onContextMenu={(event) => openFolderContextMenu(event, folder.id)}
          >
            <button
              className="file-space-tree-toggle"
              type="button"
              disabled={!hasChildren}
              onClick={() => toggleExpanded(folder.id)}
              style={{ left: `${depth * 17 - 15}px` }}
              aria-label={hasChildren ? folder.name : undefined}
            >
              {hasChildren ? (expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />) : null}
            </button>
            <button
              className="file-space-tree-name"
              type="button"
              aria-grabbed={internalFolderDrag?.folderId === folder.id}
              onPointerDown={(event) => beginFolderDragGesture(event, folder.id)}
              onClick={() => {
                if (suppressFolderClickRef.current) {
                  suppressFolderClickRef.current = false;
                  return;
                }
                chooseFolder(folder.id);
              }}
              title={folder.name}
            >
              {active ? <FolderOpen size={15} /> : <Folder size={15} />}
              <span>{folder.name}</span>
              <small>{folderCounts.get(folder.id) ?? 0}</small>
            </button>
          </div>
          {hasChildren && expanded ? renderTree(folder.id, depth + 1) : null}
        </div>
      );
    })
  );

  if (loading) {
    return (
      <section className="file-space-page file-space-page--loading" aria-busy="true">
        <span className="file-space-loading-mark" />
      </section>
    );
  }

  if (snapshot.rootStatus !== "ready") {
    const rootStatusMessage = snapshot.rootStatus === "unconfigured"
      ? null
      : t(`fileSpace.root.status.${snapshot.rootStatus}`);
    return (
      <section className="file-space-page file-space-setup-page">
        <div className="file-space-setup">
          <div className="file-space-setup-icon"><HardDrive size={28} strokeWidth={1.6} /></div>
          <p className="eyebrow">LumeTrace · {t("fileSpace.eyebrow")}</p>
          <h1>{t("fileSpace.root.title")}</h1>
          <p className="file-space-setup-description">{t("fileSpace.root.description")}</p>
          {snapshot.rootPath ? (
            <div className="file-space-invalid-path">
              <strong>{rootStatusMessage}</strong>
              <span>{snapshot.rootPath}</span>
            </div>
          ) : null}
          <div className="file-space-setup-options">
            <div>
              <button
                className="file-space-primary-button"
                type="button"
                disabled={busyAction === "configure"}
                onClick={() => void chooseStorageRoot("new")}
              >
                <FolderPlus size={16} />
                {busyAction === "configure" && rootSetupMode === "new"
                  ? t("fileSpace.root.choosingNew")
                  : t("fileSpace.root.createNew")}
              </button>
              <small>{t("fileSpace.root.createNewDescription")}</small>
            </div>
            <div>
              <button
                className="file-space-secondary-button"
                type="button"
                disabled={busyAction === "configure"}
                onClick={() => void chooseStorageRoot("import")}
              >
                <Upload size={16} />
                {busyAction === "configure" && rootSetupMode === "import"
                  ? t("fileSpace.root.importingExisting")
                  : t("fileSpace.root.importExisting")}
              </button>
              <small>{t("fileSpace.root.importExistingDescription")}</small>
            </div>
          </div>
          {setupCancelled ? <p className="file-space-setup-feedback">{t("fileSpace.root.cancel")}</p> : null}
          {error ? <p className="file-space-error" role="alert">{error}</p> : null}
          <p className="file-space-setup-privacy"><ShieldCheck size={14} />{t("fileSpace.root.privacy")}</p>
        </div>
      </section>
    );
  }

  return (
    <section className="file-space-page">
      <aside className="file-space-sidebar">
        <header className="file-space-sidebar-header">
          <p className="eyebrow">{t("fileSpace.eyebrow")}</p>
          <h1>{t("fileSpace.title")}</h1>
        </header>

        <nav className="file-space-quick-navigation" aria-label={t("fileSpace.title")}>
          <button
            className={`${!currentFolder ? "is-active" : ""}${internalFileDrag?.folderTarget?.kind === "root" ? (internalFileDrag.folderTarget.isCurrent ? " is-file-drop-current" : " is-file-drop-target") : ""}`}
            type="button"
            data-file-root-drop="true"
            onClick={() => chooseFolder(null)}
          >
            <Folder size={16} /><span>{t("fileSpace.sidebar.all")}</span><small>{snapshot.files.length}</small>
          </button>
        </nav>

        <div
          className={`file-space-folder-heading${internalFolderDrag?.target?.mode === "root" ? " is-folder-root-drop" : ""}`}
          data-folder-root-drop="true"
        >
          <span>{t("fileSpace.sidebar.folders")}</span>
          <button type="button" disabled={Boolean(busyAction)} onClick={() => openCreateFolder(null)} aria-label={t("fileSpace.sidebar.addFolder")}>
            <Plus size={15} />
          </button>
        </div>
        <div
          className={`file-space-tree${internalFolderDrag?.target?.mode === "root" ? " is-folder-root-drop" : ""}`}
          ref={folderTreeRef}
          role="tree"
          data-folder-root-drop="true"
        >
          {renderTree(null)}
        </div>

        <footer className="file-space-sidebar-footer">
          <span className="file-space-health-dot" />
          <div>
            <strong>{t("fileSpace.sidebar.statusReady")}</strong>
            <span title={snapshot.rootPath ?? ""}>{snapshot.rootPath}</span>
          </div>
        </footer>
      </aside>

      <main className="file-space-workspace">
        <header className="file-space-workspace-toolbar">
          <strong className="file-space-toolbar-title" title={workspaceTitle}>
            {workspaceTitle}
          </strong>
          <div className="file-space-preview-size-control">
            <button
              type="button"
              disabled={previewSize <= previewSizeMin}
              onClick={() => updatePreviewSize(previewSize - previewSizeStep)}
              title={t("fileSpace.toolbar.previewSmaller")}
              aria-label={t("fileSpace.toolbar.previewSmaller")}
            >
              <Minus size={16} />
            </button>
            <input
              type="range"
              min={previewSizeMin}
              max={previewSizeMax}
              step={5}
              value={previewSize}
              onInput={(event) => updatePreviewSize(Number(event.currentTarget.value))}
              aria-label={t("fileSpace.toolbar.previewSize")}
              style={{ "--preview-size-progress": `${((previewSize - previewSizeMin) / (previewSizeMax - previewSizeMin)) * 100}%` } as React.CSSProperties}
            />
            <button
              type="button"
              disabled={previewSize >= previewSizeMax}
              onClick={() => updatePreviewSize(previewSize + previewSizeStep)}
              title={t("fileSpace.toolbar.previewLarger")}
              aria-label={t("fileSpace.toolbar.previewLarger")}
            >
              <Plus size={16} />
            </button>
          </div>
          <div className="file-space-filter-menu" ref={filterMenuRef}>
            <button
              className={isFilterMenuOpen || hasActiveFilters ? "is-active" : ""}
              type="button"
              onClick={() => setFilterMenuOpen((open) => !open)}
              title={t("fileSpace.toolbar.advancedFilters")}
              aria-label={t("fileSpace.toolbar.advancedFilters")}
              aria-expanded={isFilterMenuOpen}
            >
              <Filter size={18} />
            </button>
            {filterMenuPresence.mounted ? (
              <div className={`file-space-filter-popover${filterMenuPresence.state === "open" ? " is-open" : ""}`} role="dialog" aria-label={t("fileSpace.toolbar.advancedFilters")}>
                <div className="file-space-filter-popover-heading">
                  <strong>{t("fileSpace.toolbar.advancedFilters")}</strong>
                  {hasActiveFilters ? <button type="button" onClick={resetFilters}>{t("fileSpace.toolbar.reset")}</button> : null}
                </div>
                <span>{t("fileSpace.toolbar.searchScope")}</span>
                <div className="file-space-type-filters is-wrapping">
                  {(["name", "content", "tag"] as SearchScope[]).map((scope) => (
                    <button
                      className={searchScopes.includes(scope) ? "is-active" : ""}
                      key={scope}
                      type="button"
                      aria-pressed={searchScopes.includes(scope)}
                      onClick={() => toggleSearchScope(scope)}
                    >
                      {searchScopes.includes(scope) ? <Check size={12} /> : null}
                      {t(`fileSpace.toolbar.searchScopes.${scope}`)}
                    </button>
                  ))}
                </div>
                <span>{t("fileSpace.toolbar.filterType")}</span>
                <div className="file-space-type-filters is-wrapping">
                  {(["all", "document", "image", "sheet"] as const).map((filter) => (
                    <button
                      className={typeFilter === filter ? "is-active" : ""}
                      key={filter}
                      type="button"
                      aria-pressed={typeFilter === filter}
                      onClick={() => setTypeFilter(filter)}
                    >
                      {t(`fileSpace.toolbar.filter${filter[0].toUpperCase()}${filter.slice(1)}`)}
                    </button>
                  ))}
                </div>
                <span>{t("fileSpace.toolbar.filterTag")}</span>
                <div className="file-space-type-filters is-wrapping file-space-tag-filters">
                  <button className={tagFilter === "all" ? "is-active" : ""} type="button" aria-pressed={tagFilter === "all"} onClick={() => setTagFilter("all")}>
                    {t("fileSpace.toolbar.allTags")}
                  </button>
                  {allTags.map((tag) => (
                    <button className={tagFilter === tag ? "is-active" : ""} key={tag} type="button" aria-pressed={tagFilter === tag} onClick={() => setTagFilter(tag)}>
                      {tag}
                    </button>
                  ))}
                </div>
                <span>{t("fileSpace.toolbar.filterTime")}</span>
                <div className="file-space-type-filters is-wrapping">
                  {(["all", "today", "week", "month", "year"] as TimeFilter[]).map((filter) => (
                    <button className={timeFilter === filter ? "is-active" : ""} key={filter} type="button" aria-pressed={timeFilter === filter} onClick={() => setTimeFilter(filter)}>
                      {t(`fileSpace.toolbar.time.${filter}`)}
                    </button>
                  ))}
                </div>
                <span>{t("fileSpace.toolbar.sort")}</span>
                <div className="file-space-sort-options">
                  {sortOptions.map((sort) => (
                    <button className={sortOption === sort ? "is-active" : ""} key={sort} type="button" aria-pressed={sortOption === sort} onClick={() => updateSortOption(sort)}>
                      <span>{t(`fileSpace.toolbar.sortOptions.${sort}`)}</span>
                      {sortOption === sort ? <Check size={13} /> : null}
                    </button>
                  ))}
                </div>
              </div>
            ) : null}
          </div>
          <label className="file-space-search">
            {searchLoading ? <LoaderCircle className="is-spinning" size={16} /> : <Search size={17} />}
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("fileSpace.toolbar.search")} />
            {query ? <button type="button" onClick={() => setQuery("")} aria-label={t("fileSpace.toolbar.clearSearch")}><X size={14} /></button> : null}
          </label>
        </header>

        {error ? <p className="file-space-error file-space-workspace-error" role="alert">{error}</p> : null}

        <div
          className={`file-space-content-scroll${visibleFolders.length === 0 && visibleFiles.length === 0 ? " is-empty" : ""}${isFileDragOver ? " is-drag-over" : ""}`}
          ref={contentDropZoneRef}
          aria-busy={busyAction === "import"}
          onDragEnter={handleExternalDragEnter}
          onDragOver={handleExternalDragOver}
          onDragLeave={handleExternalDragLeave}
          onDrop={handleExternalDrop}
        >
          {visibleFolders.length > 0 ? (
            <section className="file-space-content-section">
              <div className="file-space-folder-grid">
                {visibleFolders.map((folder) => (
                  <button key={folder.id} type="button" onDoubleClick={() => chooseFolder(folder.id)} onClick={() => chooseFolder(folder.id)} onContextMenu={(event) => openFolderContextMenu(event, folder.id)}>
                    <span className="file-space-folder-art">
                      <span className="file-space-folder-layer is-back" />
                      <span className="file-space-folder-layer is-middle" />
                      <span className="file-space-folder-layer is-front" />
                      <span className="file-space-folder-cover">
                        {(folderDisplayDetails.get(folder.id)?.previews ?? []).length > 0 ? (
                          <span className={`file-space-folder-preview-grid is-${Math.min(folderDisplayDetails.get(folder.id)?.previews.length ?? 0, 4)}`}>
                            {(folderDisplayDetails.get(folder.id)?.previews ?? []).map((file) => {
                              const PreviewIcon = fileIcon(file);
                              return (
                                <span className={`file-space-folder-preview is-${fileCategory(file)}`} key={file.id}>
                                  <PreviewIcon size={24} strokeWidth={1.35} />
                                  <small>{file.name.split(".").pop()?.toUpperCase().slice(0, 4)}</small>
                                </span>
                              );
                            })}
                          </span>
                        ) : null}
                      </span>
                    </span>
                    <span className="file-space-item-copy">
                      <strong>{folder.name}</strong>
                      <small>
                        {t("fileSpace.content.fileCount", { count: folderDisplayDetails.get(folder.id)?.fileCount ?? 0 })}
                        {(folderDisplayDetails.get(folder.id)?.folderCount ?? 0) > 0
                          ? ` · ${t("fileSpace.content.subfolderCount", { count: folderDisplayDetails.get(folder.id)?.folderCount ?? 0 })}`
                          : null}
                      </small>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          ) : null}

          {visibleFiles.length > 0 ? (
            <section className="file-space-content-section">
              <div
                className="file-space-file-grid"
                ref={fileGridRef}
                style={{
                  "--file-preview-size": `${previewSize}px`,
                  height: fileJustifiedLayout ? `${fileJustifiedLayout.height}px` : undefined,
                } as React.CSSProperties}
              >
                {visibleFiles.map((file) => {
                  const placement = fileJustifiedLayout?.placements[file.id];
                  return (
                    <div
                      className={`file-space-file-card${internalFileDrag?.fileId === file.id ? " is-reorder-placeholder" : ""}`}
                      data-file-id={file.id}
                      data-source-kind={file.sourceKind}
                      data-version-count={file.versionCount}
                      data-current-version={file.currentVersion ?? undefined}
                      data-preview-aspect-ratio={defaultFilePreviewAspectRatio(file)}
                      key={file.id}
                      style={{
                        width: placement ? `${placement.width}px` : undefined,
                        height: fileJustifiedLayout ? `${fileJustifiedLayout.cardHeight}px` : undefined,
                        transform: placement
                          ? `translate3d(${placement.x}px, ${placement.y}px, 0)`
                          : undefined,
                      }}
                    >
                      <button
                        type="button"
                        data-file-id={file.id}
                        title={file.name}
                        aria-grabbed={internalFileDrag?.fileId === file.id}
                        draggable={false}
                        onPointerDown={(event) => beginFileDragGesture(event, file.id)}
                        onContextMenu={(event) => openFileContextMenu(event, file.id)}
                      >
                        <FileArtwork file={file} />
                        <span className="file-space-item-copy">
                          <strong>{file.name}</strong>
                          <small>
                            {file.sourceKind === "task_artifact" ? t("fileSpace.content.taskArtifact") : t("fileSpace.content.userImport")}
                            {file.currentVersion ? ` · v${file.currentVersion}/${file.versionCount}` : ""}
                            {" · "}{formatFileSize(file.sizeBytes)}
                            {" · "}{new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(file.updatedAt)}
                          </small>
                          {file.tags.length > 0 ? (
                            <span className="file-space-card-tags">
                              {file.tags.slice(0, 3).map((tag) => <em key={tag}>{tag}</em>)}
                            </span>
                          ) : null}
                        </span>
                      </button>
                    </div>
                  );
                })}
              </div>
            </section>
          ) : null}

          {visibleFolders.length === 0 && visibleFiles.length === 0 ? (
            !showEmptyDropZone ? (
              <div className="file-space-empty file-space-empty--search">
                {searchLoading ? <LoaderCircle className="is-spinning" size={25} /> : <FolderOpen size={27} strokeWidth={1.4} />}
                <strong>{searchLoading
                  ? t("fileSpace.content.searching")
                  : t("fileSpace.content.noResults")}</strong>
              </div>
            ) : (
              <div
                className={`file-space-empty file-space-empty--drop${isFileDragOver ? " is-drag-over" : ""}`}
                role="region"
                aria-label={t("fileSpace.content.emptyDropTitle")}
                aria-busy={busyAction === "import"}
              >
                <div className="file-space-empty-illustration" aria-hidden="true">
                  <svg className="file-space-empty-thread" viewBox="0 0 350 184" preserveAspectRatio="none">
                    <path d="M64 49 C112 44 114 75 148 86" />
                    <path d="M112 136 C130 132 137 114 149 104" />
                    <path d="M201 85 C236 70 247 51 279 51" />
                    <path d="M201 105 C229 115 236 130 247 135" />
                    <circle cx="148" cy="86" r="3.5" />
                    <circle cx="149" cy="104" r="3.5" />
                    <circle cx="201" cy="85" r="3.5" />
                    <circle cx="201" cy="105" r="3.5" />
                  </svg>
                  <span className="file-space-empty-node is-document"><FileText size={21} strokeWidth={1.45} /></span>
                  <span className="file-space-empty-node is-image"><FileImage size={21} strokeWidth={1.45} /></span>
                  <span className="file-space-empty-node is-sheet"><FileSpreadsheet size={21} strokeWidth={1.45} /></span>
                  <span className="file-space-empty-node is-note"><FileText size={21} strokeWidth={1.45} /></span>
                  <span className="file-space-empty-destination">
                    <span className="file-space-empty-folder-mark">
                      <Folder size={49} strokeWidth={1.25} />
                    </span>
                  </span>
                </div>
                <strong role="status" aria-live="polite" aria-atomic="true">
                  {busyAction === "import"
                    ? t("fileSpace.content.uploading")
                    : isFileDragOver
                      ? t("fileSpace.content.emptyDropActiveTitle")
                      : t("fileSpace.content.emptyDropTitle")}
                </strong>
                <span>{t("fileSpace.content.emptyDropDescription")}</span>
                <div className="file-space-empty-actions">
                  <button type="button" disabled={Boolean(busyAction)} onClick={() => void importFiles()}>
                    <Upload size={16} />
                    {busyAction === "import" ? t("fileSpace.content.uploading") : t("fileSpace.content.uploadLocalFiles")}
                  </button>
                  <button type="button" disabled={Boolean(busyAction)} onClick={() => openCreateFolder(currentFolder?.id ?? null)}>
                    <FolderPlus size={16} />{t("fileSpace.content.createSubfolder")}
                  </button>
                </div>
              </div>
            )
          ) : null}
        </div>
        {!showEmptyDropZone && isFileDragOver ? (
          <div className="file-space-drop-feedback" role="status" aria-live="polite">
            <Upload size={18} />
            <strong>{t("fileSpace.content.dropOverlayTitle")}</strong>
          </div>
        ) : null}
        {importFeedback ? (
          <div className={`file-space-import-feedback is-${importFeedback.phase}`} role={importFeedback.phase === "failed" ? "alert" : "status"} aria-live="polite">
            <span className="file-space-import-feedback-icon">
              {["completed", "cancelled"].includes(importFeedback.phase)
                ? <Check size={16} />
                : importFeedback.phase === "failed"
                  ? <X size={16} />
                  : <LoaderCircle className="is-spinning" size={16} />}
            </span>
            <div className="file-space-import-feedback-copy">
              <strong>{t(`fileSpace.importFeedback.${importFeedback.phase}`)}</strong>
              <small>
                {importFeedback.currentName ?? (importFeedback.total > 0
                  ? t("fileSpace.importFeedback.count", { processed: importFeedback.processed, total: importFeedback.total })
                  : t("fileSpace.importFeedback.preparing"))}
              </small>
              {!["completed", "cancelled", "failed"].includes(importFeedback.phase) && importFeedback.total > 0 ? (
                <span><i style={{ width: `${Math.min(100, (importFeedback.processed / importFeedback.total) * 100)}%` }} /></span>
              ) : null}
            </div>
            {["scanning", "importing", "cancelling"].includes(importFeedback.phase) ? (
              <button type="button" disabled={importFeedback.phase === "cancelling"} onClick={() => void cancelImport()}>{t("fileSpace.importFeedback.cancel")}</button>
            ) : (
              <button type="button" onClick={() => setImportFeedback(null)} aria-label={t("fileSpace.importFeedback.dismiss")}><X size={15} /></button>
            )}
          </div>
        ) : null}
        {fileMoveFeedback ? (
          <div className="file-space-import-feedback is-completed" role="status" aria-live="polite">
            <span className="file-space-import-feedback-icon"><Check size={16} /></span>
            <div className="file-space-import-feedback-copy">
              <strong>{t(`fileSpace.moveFeedback.${fileMoveFeedback.status}Title`)}</strong>
              <small>{t(`fileSpace.moveFeedback.${fileMoveFeedback.status}Description`, {
                fileName: fileMoveFeedback.fileName,
                destinationName: fileMoveFeedback.destinationName,
              })}</small>
            </div>
            <button type="button" onClick={() => setFileMoveFeedback(null)} aria-label={t("fileSpace.moveFeedback.dismiss")}><X size={15} /></button>
          </div>
        ) : null}
      </main>

      {timeline && timelineFile ? (
        <div className="file-space-timeline-backdrop" onMouseDown={(event) => {
          if (event.target === event.currentTarget) {
            setTimeline(null);
            setTimelineFile(null);
          }
        }}>
          <aside className="file-space-timeline-panel" aria-label={t("fileSpace.timeline.ariaLabel")}>
            <header>
              <div>
                <span>{t("fileSpace.timeline.taskArtifact")} · {timeline.logicalKey}</span>
                <h2>{timelineFile.name}</h2>
              </div>
              <button type="button" onClick={() => { setTimeline(null); setTimelineFile(null); }} aria-label={t("fileSpace.timeline.close")}><X size={18} /></button>
            </header>
            <div className="file-space-timeline-body">
              <section className="file-space-version-list">
                {timeline.versions.map((version) => (
                  <div
                    key={version.id}
                    className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}
                  >
                    <button type="button" onClick={() => void selectTimelineVersion(version)}>
                      <strong>v{version.versionNumber}{version.isCurrent ? ` · ${t("fileSpace.timeline.current")}` : ""}</strong>
                      <span>{version.origin === "task" ? version.taskTitle : t("fileSpace.timeline.userEdit")}</span>
                      {version.roundNumber ? <span>{t("fileSpace.timeline.round", { count: version.roundNumber })} · {version.cellName}</span> : null}
                      <small>{new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(version.producedAt)}</small>
                    </button>
                    {!version.isCurrent ? (
                      <button type="button" disabled={timelineBusy} onClick={() => void setCurrentTimelineVersion(version.id)}>{t("fileSpace.timeline.setCurrent")}</button>
                    ) : null}
                  </div>
                ))}
              </section>
              <section className="file-space-version-preview">
                <h3>{t("fileSpace.timeline.historicalContent")}</h3>
                {timelineBusy ? <p>{t("fileSpace.timeline.loading")}</p> : versionPreview !== null ? <pre>{versionPreview}</pre> : <p>{t("fileSpace.timeline.noTextPreview")}</p>}
                <h3>{t("fileSpace.timeline.operations")}</h3>
                <ol>
                  {timeline.events.map((event) => {
                    const translationKey = eventTranslationKey(event.eventType);
                    return (
                      <li key={event.id}>
                        <strong>{translationKey ? t(translationKey) : event.eventType}</strong>
                        <span>{t(`fileSpace.timeline.actors.${event.actor}`)} · {new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(event.createdAt)}</span>
                      </li>
                    );
                  })}
                </ol>
              </section>
            </div>
          </aside>
        </div>
      ) : null}

      {internalFolderDrag && folderBeingDragged ? createPortal(
        <div
          className="file-space-folder-drag-overlay"
          aria-hidden="true"
          style={{
            left: `${internalFolderDrag.pointerX - internalFolderDrag.offsetX}px`,
            top: `${internalFolderDrag.pointerY - internalFolderDrag.offsetY}px`,
            width: `${internalFolderDrag.rowWidth}px`,
            height: `${internalFolderDrag.rowHeight}px`,
          }}
        >
          <FolderOpen size={15} />
          <span>{folderBeingDragged.name}</span>
          <small>{folderCounts.get(folderBeingDragged.id) ?? 0}</small>
        </div>,
        document.body,
      ) : null}

      {internalFileDrag && internallyDraggedFile ? createPortal(
        <div
          className="file-space-file-reorder-overlay"
          aria-hidden="true"
          style={{
            left: `${internalFileDrag.pointerX - internalFileDrag.offsetX}px`,
            top: `${internalFileDrag.pointerY - internalFileDrag.offsetY}px`,
            width: `${internalFileDrag.cardWidth}px`,
            height: `${internalFileDrag.cardHeight}px`,
            "--file-preview-size": `${previewSize}px`,
          } as React.CSSProperties}
        >
          <FileArtwork file={internallyDraggedFile} />
          <span className="file-space-item-copy">
            <strong>{internallyDraggedFile.name}</strong>
            <small>
              {internallyDraggedFile.sourceKind === "task_artifact" ? t("fileSpace.content.taskArtifact") : t("fileSpace.content.userImport")}
              {internallyDraggedFile.currentVersion ? ` · v${internallyDraggedFile.currentVersion}/${internallyDraggedFile.versionCount}` : ""}
              {" · "}{formatFileSize(internallyDraggedFile.sizeBytes)}
              {" · "}{new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(internallyDraggedFile.updatedAt)}
            </small>
            {internallyDraggedFile.tags.length > 0 ? (
              <span className="file-space-card-tags">
                {internallyDraggedFile.tags.slice(0, 3).map((tag) => <em key={tag}>{tag}</em>)}
              </span>
            ) : null}
          </span>
        </div>,
        document.body,
      ) : null}

      {folderContextMenu ? (
        <div
          className="file-space-context-menu"
          ref={contextMenuRef}
          role="menu"
          style={{ left: folderContextMenu.x, top: folderContextMenu.y }}
        >
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => openCreateFolder(folderContextMenu.folderId)}>
            <FolderPlus size={16} />{t("fileSpace.folderMenu.newSubfolder")}
          </button>
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => openRenameFolder(folderContextMenu.folderId)}>
            <Pencil size={15} />{t("fileSpace.folderMenu.rename")}
          </button>
          <span />
          <button className="is-danger" type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => requestDeleteFolder(folderContextMenu.folderId)}>
            <Trash2 size={16} />{t("fileSpace.folderMenu.delete")}
          </button>
        </div>
      ) : null}

      {fileContextMenu ? (
        <div
          className="file-space-context-menu file-space-file-context-menu"
          ref={contextMenuRef}
          role="menu"
          style={{ left: fileContextMenu.x, top: fileContextMenu.y }}
        >
          {(snapshot.files.find((file) => file.id === fileContextMenu.fileId)?.versionCount ?? 0) > 0 ? (
            <>
              <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => {
                const file = snapshot.files.find((item) => item.id === fileContextMenu.fileId);
                if (file) void openTimeline(file);
              }}>
                <Clock3 size={16} />{t("fileSpace.fileMenu.versionHistory")}
              </button>
              <span />
            </>
          ) : null}
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => void runFileCommand("reveal_file_space_file", fileContextMenu.fileId, "revealFile")}>
            <FolderOpen size={16} />{t("fileSpace.fileMenu.reveal")}
          </button>
          <span />
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => openRenameFile(fileContextMenu.fileId)}>
            <Pencil size={15} />{t("fileSpace.fileMenu.rename")}
          </button>
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => openTagDialog(fileContextMenu.fileId)}>
            <Tag size={15} />{t("fileSpace.fileMenu.tags")}
          </button>
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => void runFileCommand("copy_file_space_file", fileContextMenu.fileId, "copyFile")}>
            <Copy size={15} />{t("fileSpace.fileMenu.copyFile")}
          </button>
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => void runFileCommand("copy_file_space_file_path", fileContextMenu.fileId, "copyPath")}>
            <FileText size={15} />{t("fileSpace.fileMenu.copyPath")}
          </button>
          <span />
          <button className="is-danger" type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => requestDeleteFile(fileContextMenu.fileId)}>
            <Trash2 size={16} />{t("fileSpace.fileMenu.delete")}
          </button>
        </div>
      ) : null}

      {createDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${createDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setCreateFolderOpen(false);
          }}
        >
          <form className={`file-space-dialog${createDialogPresence.state === "open" ? " is-open" : ""}`} onSubmit={(event) => { event.preventDefault(); void createFolder(); }}>
            <header>
              <div><h2>{t("fileSpace.createFolder.title")}</h2><p>{t("fileSpace.createFolder.description")}</p></div>
              <button type="button" onClick={() => setCreateFolderOpen(false)} aria-label={t("fileSpace.createFolder.cancel")}><X size={17} /></button>
            </header>
            <label>
              <span>{t("fileSpace.createFolder.label")}</span>
              <input ref={folderNameRef} value={folderName} onChange={(event) => setFolderName(event.target.value)} placeholder={t("fileSpace.createFolder.placeholder")} />
            </label>
            <footer>
              <button type="button" onClick={() => setCreateFolderOpen(false)}>{t("fileSpace.createFolder.cancel")}</button>
              <button className="is-primary" type="submit" disabled={!folderName.trim() || Boolean(busyAction)}>
                {busyAction === "create" ? t("fileSpace.createFolder.submitting") : t("fileSpace.createFolder.submit")}
              </button>
            </footer>
          </form>
        </div>
      ) : null}

      {renameDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${renameDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setRenameFolderId(null);
          }}
        >
          <form className={`file-space-dialog${renameDialogPresence.state === "open" ? " is-open" : ""}`} onSubmit={(event) => { event.preventDefault(); void renameFolder(); }}>
            <header>
              <div><h2>{t("fileSpace.renameFolder.title")}</h2><p>{t("fileSpace.renameFolder.description")}</p></div>
              <button type="button" onClick={() => setRenameFolderId(null)} aria-label={t("fileSpace.renameFolder.cancel")}><X size={17} /></button>
            </header>
            <label>
              <span>{t("fileSpace.renameFolder.label")}</span>
              <input ref={renameFolderNameRef} value={renameFolderName} onChange={(event) => setRenameFolderName(event.target.value)} />
            </label>
            <footer>
              <button type="button" onClick={() => setRenameFolderId(null)}>{t("fileSpace.renameFolder.cancel")}</button>
              <button className="is-primary" type="submit" disabled={!renameFolderName.trim() || Boolean(busyAction)}>
                {busyAction === "rename" ? t("fileSpace.renameFolder.submitting") : t("fileSpace.renameFolder.submit")}
              </button>
            </footer>
          </form>
        </div>
      ) : null}

      {renameFileDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${renameFileDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setRenameFileId(null);
          }}
        >
          <form className={`file-space-dialog${renameFileDialogPresence.state === "open" ? " is-open" : ""}`} onSubmit={(event) => { event.preventDefault(); void renameFile(); }}>
            <header>
              <div><h2>{t("fileSpace.renameFile.title")}</h2><p>{t("fileSpace.renameFile.description")}</p></div>
              <button type="button" onClick={() => setRenameFileId(null)} aria-label={t("fileSpace.renameFile.cancel")}><X size={17} /></button>
            </header>
            <label>
              <span>{t("fileSpace.renameFile.label")}</span>
              <input ref={renameFileNameRef} value={renameFileName} onChange={(event) => setRenameFileName(event.target.value)} />
            </label>
            <footer>
              <button type="button" onClick={() => setRenameFileId(null)}>{t("fileSpace.renameFile.cancel")}</button>
              <button className="is-primary" type="submit" disabled={!renameFileName.trim() || Boolean(busyAction)}>
                {busyAction === "rename" ? t("fileSpace.renameFile.submitting") : t("fileSpace.renameFile.submit")}
              </button>
            </footer>
          </form>
        </div>
      ) : null}

      {tagDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${tagDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setTagFileId(null);
          }}
        >
          <form className={`file-space-dialog${tagDialogPresence.state === "open" ? " is-open" : ""}`} onSubmit={(event) => { event.preventDefault(); void saveFileTags(); }}>
            <header>
              <div><h2>{t("fileSpace.tags.title")}</h2><p>{t("fileSpace.tags.description")}</p></div>
              <button type="button" onClick={() => setTagFileId(null)} aria-label={t("fileSpace.tags.cancel")}><X size={17} /></button>
            </header>
            <label>
              <span>{t("fileSpace.tags.label")}</span>
              <input ref={tagDraftRef} value={tagDraft} onChange={(event) => setTagDraft(event.target.value)} placeholder={t("fileSpace.tags.placeholder")} />
              <small>{t("fileSpace.tags.hint")}</small>
            </label>
            <footer>
              <button type="button" onClick={() => setTagFileId(null)}>{t("fileSpace.tags.cancel")}</button>
              <button className="is-primary" type="submit" disabled={Boolean(busyAction)}>
                {busyAction === "fileAction" ? t("fileSpace.tags.submitting") : t("fileSpace.tags.submit")}
              </button>
            </footer>
          </form>
        </div>
      ) : null}

      {deleteDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${deleteDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setDeleteFolderId(null);
          }}
        >
          <div className={`file-space-dialog file-space-delete-dialog${deleteDialogPresence.state === "open" ? " is-open" : ""}`} role="alertdialog" aria-modal="true">
            <header>
              <div><h2>{t("fileSpace.deleteFolder.title")}</h2><p>{t("fileSpace.deleteFolder.description", { name: snapshot.folders.find((folder) => folder.id === deleteFolderId)?.name ?? "" })}</p></div>
              <button type="button" onClick={() => setDeleteFolderId(null)} aria-label={t("fileSpace.deleteFolder.cancel")}><X size={17} /></button>
            </header>
            <footer>
              <button type="button" onClick={() => setDeleteFolderId(null)}>{t("fileSpace.deleteFolder.cancel")}</button>
              <button className="is-danger" type="button" disabled={Boolean(busyAction)} onClick={() => void deleteFolder()}>
                {busyAction === "delete" ? t("fileSpace.deleteFolder.submitting") : t("fileSpace.deleteFolder.submit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {deleteFileDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${deleteFileDialogPresence.state === "open" ? " is-open" : ""}`}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setDeleteFileId(null);
          }}
        >
          <div className={`file-space-dialog file-space-delete-dialog${deleteFileDialogPresence.state === "open" ? " is-open" : ""}`} role="alertdialog" aria-modal="true">
            <header>
              <div>
                <h2>{t("fileSpace.deleteFile.title")}</h2>
                <p>{t("fileSpace.deleteFile.description", { name: deletedFile?.name ?? "" })}</p>
              </div>
              <button type="button" onClick={() => setDeleteFileId(null)} aria-label={t("fileSpace.deleteFile.cancel")}><X size={17} /></button>
            </header>
            <footer>
              <button type="button" onClick={() => setDeleteFileId(null)}>{t("fileSpace.deleteFile.cancel")}</button>
              <button className="is-danger" type="button" disabled={Boolean(busyAction)} onClick={() => void deleteFile()}>
                {busyAction === "delete" ? t("fileSpace.deleteFile.submitting") : t("fileSpace.deleteFile.submit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}
    </section>
  );
}
