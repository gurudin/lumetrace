import { useWorkspaceInvoke } from "../../shared/extensions/useWorkspaceInvoke";
import { fileMimeType } from "../../shared/extensions/fileMimeType";
import type { WorkspaceCommandSource } from "../../shared/extensions/workspaceCommands";
import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { toBlob } from "html-to-image";
import {
  ArrowDownWideNarrow,
  ArrowUpDown,
  ArrowUpNarrowWide,
  ChevronLeft,
  ChevronRight,
  Check,
  CircleAlert,
  Clock3,
  Copy,
  ChevronsDownUp,
  ChevronsUpDown,
  File,
  FileImage,
  FilePlus2,
  FileSpreadsheet,
  FileText,
  Filter,
  Folder,
  FolderPlus,
  FolderOpen,
  LoaderCircle,
  LayoutGrid,
  Minus,
  Pencil,
  PanelLeft,
  Plus,
  Search,
  ShieldCheck,
  Tag,
  Trash2,
  Upload,
  Info,
  X,
} from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import lumeTraceLogo from "../../../src-tauri/icons/icon.png";
import { useTheme } from "../../shared/theme/useTheme";
import { dismissStartupSplash } from "../../shared/ui/startupSplash";
import { usePresence } from "../../shared/ui/usePresence";
import {
  FileSpaceAiSurface,
  type FileSpaceAiSourceReference,
} from "./FileSpaceAiSurface";
import { FileSpaceBackgroundStatusButton } from "./FileSpaceBackgroundStatusButton";
import { FileSpaceInspector } from "./FileSpaceInspector";
import {
  FileSpaceSearchPanel,
  type FileSpaceSearchFile,
  type FileSpaceSearchMatch,
} from "./FileSpaceSearchPanel";
import { FileSpaceSettingsMenu } from "./FileSpaceSettingsMenu";
import { FileSpaceTrashView, type FileSpaceTrashItemRecord } from "./FileSpaceTrashView";
import {
  FileSpaceWorkspaceSwitcher,
  type FileSpaceWorkspaceDirectory,
  type FileSpaceWorkspaceMutation,
} from "./FileSpaceWorkspaceMenu";
import { FileVersionDiff, type VersionDiffStatus } from "./FileVersionDiff";
import { VersionAnnotation, useVersionAnnotationUpdates } from "./VersionAnnotation";
import { VersionHistoryBadge, type VersionSummary } from "./VersionHistoryBadge";
import { ImportExistingFolderSheet } from "./ImportExistingFolderSheet";
import { SetupPreferences } from "./SetupPreferences";
import { ApplicationExtensionEntry, useApplicationExtension, useWorkspaceExtension } from "../../shared/extensions/ApplicationExtension";
import { globalSearchShortcutLabel } from "./globalSearchShortcut";
import {
  calculateFileListLayout,
  calculateJustifiedFileLayout,
  justifiedFileLayoutsEqual,
  reorderFileIdsForDraggedCard,
  visibleJustifiedFileIds,
  type JustifiedFileLayout,
} from "./fileJustifiedLayout";
import {
  fileKeyboardShortcutAction,
  nextFileIdForKeyboard,
  type FileKeyboardDirection,
} from "./fileKeyboardNavigation";
import { resolveFileDoubleClickRoute } from "./fileOpenRouting";
import { FileRevealNavigation } from "./fileRevealNavigation";
import {
  fileCardMetadataText,
  originalImageDimensions,
  fileDocumentArtworkFormat,
  shouldShowFileVersionBadge,
  type FileDocumentArtworkFormat,
  type FileImageDimensions,
} from "./fileCardPresentation";
import {
  combineMarqueeSelection,
  normalizeSelectionRectangle,
  orderedSelection,
  pointInScrollContent,
  pruneSelection,
  rectanglesIntersect,
  resolveFileClickSelection,
  selectionVisibilityWithPendingReveal,
  setsEqual,
  type FileSelectionMode,
} from "./fileSelection";
import {
  canActivateFileReorder,
  decideFileDragHandoff,
  hasMeaningfulFileReorderMovement,
} from "./fileDragRouting";
import {
  expandableFolderIdsInSubtree,
  isFolderSubtreeFullyExpanded,
  setFolderSubtreeExpanded,
  toggleFolderOnDoubleClick,
} from "./folderTreeExpansion";
import {
  defaultVersionComparison,
  diffTextVersions,
  type VersionDiffResult,
} from "./versionDiff";
import {
  mergeVersionNotifications,
  type FileSpaceVersionNotification,
} from "./versionNotification";
import { claimFirstVersionChange, viewVersionChangeEvent } from "./firstVersionChange";
import { InitialVersionHint, VersionName } from "./InitialVersionHint";
import {
  inspectorVisibilityStorageKey,
  storedInspectorVisibility,
} from "./fileSpaceViewPreferences";
import { fileSpaceRootView } from "./fileSpaceRootPresentation";

interface FileSpaceFolderRecord {
  id: string;
  parentId: string | null;
  name: string;
  relativePath: string;
  manualOrder: number;
  directFileCount?: number;
  fileCount?: number;
  childFolderCount?: number;
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

interface TaskFileEventRecord {
  id: string;
  versionId: string | null;
  eventType: string;
  actor: "task" | "user";
  details: Record<string, unknown>;
  createdAt: number;
}

interface TaskFileTimelineRecord {
  workspaceId?: string;
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
  fileCount: number;
  tags: string[];
  folders: FileSpaceFolderRecord[];
  files: FileSpaceFileRecord[];
  trashItems: FileSpaceTrashItemRecord[];
}

interface FileSpaceFilePageCursor {
  sort: SortOption;
  number: number | null;
  text: string | null;
  secondaryText: string | null;
  id: string;
}

interface FileSpaceFilePage {
  files: FileSpaceFileRecord[];
  totalCount: number;
  nextCursor: FileSpaceFilePageCursor | null;
}

interface FileSpaceCreatedFileResult {
  file: FileSpaceFileRecord;
  snapshot: FileSpaceSnapshot;
}

type FileSpaceCollection = "files" | "trash";
type TypeFilter = "all" | "document" | "image" | "sheet";
type SearchScope = "name" | "content" | "tag";
type TimeFilter = "all" | "today" | "week" | "month" | "year";
type SortOption = "manual" | "updatedDesc" | "updatedAsc" | "nameAsc" | "nameDesc" | "sizeDesc" | "typeAsc";
type FileLayoutMode = "adaptive" | "list";
type FolderContextMenu = { folderId: string; x: number; y: number };
type FileContextMenu = { fileId: string; x: number; y: number };
type ContentContextMenu = { x: number; y: number };
type NewTextFileFormat = "md" | "txt";
type FileSpaceBusyAction = "configure" | "create" | "rename" | "delete" | "restore" | "purgeTrash" | "import" | "fileAction" | "reorder" | "moveFolder" | "moveFile";
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
  fileName: string | null;
  fileCount: number;
  destinationName: string;
  status: "moved" | "unchanged";
}

interface FileMoveFailure {
  fileId: string;
  message: string;
}

interface FileMoveResult {
  snapshot: FileSpaceSnapshot;
  movedIds: string[];
  unchangedIds: string[];
  failed: FileMoveFailure[];
}

interface FileSpaceTrashPurgeResult {
  snapshot: FileSpaceSnapshot;
  purgedCount: number;
  failedCount: number;
  failureMessage: string | null;
}

interface BrowserDroppedFile {
  file: File;
  relativePath: string;
}

interface FileSpaceDroppedFilePayload {
  relativePath: string;
  bytes: number[];
}

interface FileSpaceDroppedFileConflict {
  relativePath: string;
  fileName: string;
  existingFileId: string;
  existingVersion: number;
  existingSizeBytes: number;
  incomingSizeBytes: number;
  identical: boolean;
}

interface FileSpaceDroppedFileConflictInspection {
  conflicts: FileSpaceDroppedFileConflict[];
}

interface IdenticalImportFile {
  fileId: string;
  fileName: string;
}

interface IdenticalImportNotice {
  files: IdenticalImportFile[];
}

interface PendingFileImportConflict {
  requestId: string;
  folderId: string | null;
  source:
    | { kind: "paths"; paths: string[] }
    | { kind: "droppedFiles"; files: FileSpaceDroppedFilePayload[] };
  conflicts: FileSpaceDroppedFileConflict[];
  identicalFiles: IdenticalImportFile[];
}

type FileImportConflictAction = "rename" | "latestVersion";

interface FileDragGesture {
  fileId: string;
  fileIds: string[];
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
  preview: PreparedFileDragPreview | null;
}

interface PreparedFileDragPreview {
  immediateBytes: number[] | null;
  promise: Promise<number[] | null>;
}

interface InternalFileDrag {
  fileId: string;
  fileIds: string[];
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

interface FileMarqueeGesture {
  pointerId: number;
  startX: number;
  startY: number;
  startClientX: number;
  startClientY: number;
  lastClientX: number;
  lastClientY: number;
  baselineIds: Set<string>;
  baselineAnchorId: string | null;
  mode: FileSelectionMode;
  phase: "pending" | "selecting";
}

interface FileSelectionMarquee {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface PointerSelectionIntent {
  fileId: string;
  collapseOnClick: boolean;
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
  fileCount: 0,
  tags: [],
  folders: [],
  files: [],
  trashItems: [],
};
const currentFolderStorageKeyBase = "lumetrace.file-space.current-folder";
const previewSizeStorageKey = "lumetrace.file-space.preview-size";
const sortOptionStorageKey = "lumetrace.file-space.sort-option";
const fileLayoutModeStorageKey = "lumetrace.file-space.layout-mode";
const trashRetentionMs = 30 * 24 * 60 * 60 * 1000;
const maxTrashCleanupTimerMs = 2_147_000_000;
const folderDragThreshold = 5;
const previewSizeMin = 120;
const previewSizeMax = 260;
const previewSizeStep = 10;
const previewSizeDefault = 155;
const fileLayoutHorizontalGap = 15;
const fileLayoutVerticalGap = 18;
const filePreviewDetailsGap = 9;
const filePageLimit = 160;
const fileVirtualOverscan = 900;
const fileDetailsHeightFallback = 38;
const fileListPreviewSize = 42;
const fileListRowHeight = 52;
const fileListVerticalGap = 2;
const fileDragThreshold = 6;
const fileReorderCommitThreshold = 24;
const fileMarqueeThreshold = 5;
const fileMarqueeScrollInset = 34;
const fileMarqueeMaxScrollSpeed = 18;
const fileDragWindowExitMargin = 1;
const fileDragPreviewMaxSize = 160;
const maxVersionDiffTextBytes = 5 * 1024 * 1024;

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

function fallbackFileDragPreviewBytes(target: HTMLButtonElement) {
  const artwork = target.querySelector<HTMLElement>(".file-space-file-art");
  if (!artwork) return null;
  const { width, height } = artwork.getBoundingClientRect();
  if (width <= 0 || height <= 0) return null;
  const scale = Math.min(1, fileDragPreviewMaxSize / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width * scale));
  canvas.height = Math.max(1, Math.round(height * scale));
  const context = canvas.getContext("2d");
  if (!context) return null;

  const dark = document.documentElement.dataset.theme === "dark";
  const inset = Math.max(1, Math.round(Math.min(canvas.width, canvas.height) * 0.025));
  const radius = Math.max(7, Math.round(Math.min(canvas.width, canvas.height) * 0.065));
  const extension = target.title.includes(".")
    ? target.title.split(".").pop()?.toUpperCase().slice(0, 5) ?? "FILE"
    : "FILE";
  const label = artwork.querySelector<HTMLElement>(
    ".file-space-format-kicker, .file-space-archive-document > em, small",
  )?.textContent?.trim() || extension;
  const title = artwork.querySelector<HTMLElement>(".file-space-format-title")
    ?.textContent?.trim() || target.title.replace(/\.[^.]+$/, "");

  try {
    context.beginPath();
    context.roundRect(
      inset,
      inset,
      canvas.width - inset * 2,
      canvas.height - inset * 2,
      radius,
    );
    context.fillStyle = dark ? "#303136" : "#f7f7f6";
    context.fill();
    context.lineWidth = Math.max(1, Math.round(scale));
    context.strokeStyle = dark ? "#515259" : "#d6d6d8";
    context.stroke();

    const padding = Math.max(10, Math.round(canvas.width * 0.1));
    context.fillStyle = dark ? "#d6b3bf" : "#98697a";
    context.font = `700 ${Math.max(11, Math.round(canvas.height * 0.09))}px -apple-system, BlinkMacSystemFont, sans-serif`;
    context.textBaseline = "top";
    context.fillText(label.slice(0, 5), padding, padding, canvas.width - padding * 2);

    context.fillStyle = dark ? "#ececef" : "#343438";
    context.font = `600 ${Math.max(10, Math.round(canvas.height * 0.075))}px -apple-system, BlinkMacSystemFont, sans-serif`;
    const titleY = Math.round(canvas.height * 0.36);
    const maxTitleWidth = canvas.width - padding * 2;
    const originalTitle = title || target.title || extension;
    let displayTitle = originalTitle;
    while (displayTitle.length > 1 && context.measureText(displayTitle).width > maxTitleWidth) {
      displayTitle = displayTitle.slice(0, -1);
    }
    if (displayTitle !== originalTitle && displayTitle.length > 1) {
      displayTitle = `${displayTitle.slice(0, -1)}…`;
    }
    context.fillText(displayTitle, padding, titleY, maxTitleWidth);

    context.fillStyle = dark ? "#5c5d63" : "#d9d9dc";
    const lineHeight = Math.max(3, Math.round(canvas.height * 0.025));
    const lineGap = Math.max(6, Math.round(canvas.height * 0.065));
    const lineTop = Math.round(canvas.height * 0.62);
    [1, 0.78, 0.9].forEach((lineScale, index) => {
      context.fillRect(
        padding,
        lineTop + index * lineGap,
        Math.round(maxTitleWidth * lineScale),
        lineHeight,
      );
    });
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

function prepareFileDragPreview(target: HTMLButtonElement): PreparedFileDragPreview {
  const immediateBytes = imageDragPreviewBytes(target) ?? fallbackFileDragPreviewBytes(target);
  const preview: PreparedFileDragPreview = {
    immediateBytes,
    promise: Promise.resolve(immediateBytes),
  };
  preview.promise = fileDragPreviewBytes(target).then((bytes) => {
    if (bytes) preview.immediateBytes = bytes;
    return bytes ?? preview.immediateBytes;
  });
  return preview;
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
  return value && sortOptions.includes(value) ? value : "manual";
}

function storedFileLayoutMode(): FileLayoutMode {
  return window.localStorage.getItem(fileLayoutModeStorageKey) === "list" ? "list" : "adaptive";
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
  const candidateIds = new Set<string>();
  const candidates = orderedIds.flatMap((id) => {
    if (id === draggedId) return [];
    const card = grid.querySelector<HTMLElement>(`.file-space-file-card[data-file-id="${CSS.escape(id)}"]`);
    if (!card) return [];
    candidateIds.add(id);
    const rect = card.getBoundingClientRect();
    return [{
      id,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
    }];
  });
  const windowIds = orderedIds.filter((id) => id === draggedId || candidateIds.has(id));
  if (windowIds.length < 2) return orderedIds;
  const reorderedWindowIds = reorderFileIdsForDraggedCard({
    orderedIds: windowIds,
    draggedId,
    candidates,
    draggedCenterX: clientX - offsetX + cardWidth / 2,
    draggedCenterY: clientY - offsetY + cardHeight / 2,
  });
  const windowIdSet = new Set(windowIds);
  let windowIndex = 0;
  return orderedIds.map((id) => (
    windowIdSet.has(id) ? reorderedWindowIds[windowIndex++] : id
  ));
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
  const trashItems = Array.from({ length: 12 }, (_, index): FileSpaceTrashItemRecord => ({
    id: `fixture-trash-${index}`,
    rootId: `fixture-trash-root-${index}`,
    itemType: index % 3 === 0 ? "folder" : "file",
    name: index % 3 === 0
      ? `归档项目 ${String(index + 1).padStart(2, "0")}`
      : `已删除的研究资料-${String(index + 1).padStart(2, "0")}.${extensions[index % extensions.length]}`,
    originalRelativePath: index % 3 === 0
      ? `Clipboard X/归档项目 ${String(index + 1).padStart(2, "0")}`
      : `Clipboard X/市场调研/已删除的研究资料-${String(index + 1).padStart(2, "0")}.${extensions[index % extensions.length]}`,
    sizeBytes: (index + 1) * 148_000,
    fileCount: index % 3 === 0 ? index + 2 : 1,
    folderCount: index % 3 === 0 ? 2 + (index % 4) : 0,
    versionCount: index % 3 === 0 ? index + 2 : 1 + (index % 4),
    trashedAt: now - index * 4_200_000,
  }));
  return {
    rootPath: "/Users/example/Documents/Lume Trace",
    rootName: "Lume Trace",
    rootStatus: "ready",
    fileCount: files.length,
    tags: [...new Set(files.flatMap((file) => file.tags))],
    folders,
    files,
    trashItems,
  };
}

function createVisualTimeline(file: FileSpaceFileRecord): TaskFileTimelineRecord | null {
  if (file.versionCount < 1) return null;
  const versions = Array.from({ length: file.versionCount }, (_, index): TaskFileVersionRecord => {
    const versionNumber = file.versionCount - index;
    return {
      id: `${file.id}-version-${versionNumber}`,
      versionNumber,
      name: file.name,
      mimeType: file.mimeType,
      sizeBytes: file.sizeBytes,
      origin: versionNumber % 2 === 0 ? "user_edit" : "task",
      taskId: versionNumber % 2 === 0 ? null : "fixture-task",
      taskTitle: versionNumber % 2 === 0 ? null : "产品调研任务",
      roundNumber: versionNumber % 2 === 0 ? null : versionNumber,
      cellId: versionNumber % 2 === 0 ? null : "fixture-cell",
      cellName: versionNumber % 2 === 0 ? null : "Research Cell",
      producedAt: file.updatedAt - index * 86_400_000,
      isCurrent: versionNumber === file.currentVersion,
    };
  });
  return {
    fileId: file.id,
    logicalKey: file.relativePath,
    workspaceId: "visual-preview",
    currentVersionId: versions.find((version) => version.isCurrent)?.id ?? versions[0].id,
    versions,
    events: [],
  };
}

function createVisualVersionContent(file: FileSpaceFileRecord, version: TaskFileVersionRecord) {
  if (import.meta.env.DEV && file.id === "fixture-timeline-scroll") {
    return Array.from({ length: 160 }, (_, index) => (
      `Version ${version.versionNumber} / line ${index + 1}: ${"Synthetic preview content. ".repeat(6)}`
    )).join("\n");
  }
  const lines = [
    `# ${file.name}`,
    "",
    `版本：v${version.versionNumber}`,
    "状态：整理中",
    "",
    "## 研究目标",
    "确认目标用户最常见的文件版本管理问题。",
    "",
    "## 结论",
    "文件需要保留来源、修改历史和当前版本。",
  ];
  if (version.versionNumber >= 2) {
    lines[3] = "状态：已完成初步验证";
    lines.push("", "新增：版本比较需要明确标记增加、删除和修改内容。");
  }
  if (version.versionNumber >= 3) {
    lines[6] = "确认小团队在共享文件时最常见的版本冲突问题。";
    lines.push("新增：比较结果必须能够定位到具体行。", "待办：补充真实用户案例。");
  }
  return `${lines.join("\n")}\n`;
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
  const mime = file.mimeType ?? fileMimeType(file.name) ?? "";
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
    ["md", "markdown", "txt", "doc", "docx", "pdf", "ppt", "pptx"].includes(extension)
  ) return "document";
  return "other";
}

function FileCardMetadata({ file, imageDimensions }: { file: FileSpaceFileRecord; imageDimensions?: FileImageDimensions }) {
  const source = useWorkspaceExtension()?.source ?? null;
  const image = fileCategory(file) === "image";
  // The transport identity includes external revisions (such as ETags), which
  // can change even when the file timestamp remains the same.
  const key = JSON.stringify([source?.key, file.id, file.updatedAt, image ? filePreviewSource(file, source) : null]);
  const [original, setOriginal] = useState<{ key: string; dimensions: FileImageDimensions } | null>(null);
  // Let lazy thumbnails paint first. Merely mounting/laying out the catalogue
  // must not enqueue metadata for every image, including offscreen cards.
  const thumbnailReady = imageDimensions?.fileUpdatedAt === file.updatedAt;
  useEffect(() => {
    let current = true;
    if (source?.imageOriginalDimensions && image && thumbnailReady) {
      void source.imageOriginalDimensions(file.id, file.updatedAt).then(value => {
        const dimensions = originalImageDimensions(value);
        if (current) setOriginal(dimensions ? { key, dimensions: { ...dimensions, fileUpdatedAt: file.updatedAt } } : null);
      }).catch(() => { if (current) setOriginal(null); });
    }
    return () => { current = false; };
  }, [source, key, image, thumbnailReady, file.id, file.updatedAt]);
  // Intrinsic preview sizes still drive layout. Only local previews contain the
  // original file; an external thumbnail must never become a resolution label.
  const dimensions = source ? (original?.key === key ? original.dimensions : undefined) : imageDimensions;
  return fileCardMetadataText(
    formatFileSize(file.sizeBytes),
    fileCategory(file) === "image",
    file.updatedAt,
    dimensions,
  );
}

function imageDimensionsKey(fileId: string, fileUpdatedAt: number) {
  return `${fileId}:${fileUpdatedAt}`;
}

function fileIcon(file: FileSpaceFileRecord) {
  const category = fileCategory(file);
  if (category === "image") return FileImage;
  if (category === "sheet") return FileSpreadsheet;
  if (category === "document") return FileText;
  return File;
}

type FileArtworkFormat = FileDocumentArtworkFormat | "archive";
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
  return fileDocumentArtworkFormat(extension);
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

function filePreviewSource(file: FileSpaceFileRecord, source: WorkspaceCommandSource | null, purpose: "thumbnail" | "detail" = "thumbnail") {
  if (fileCategory(file) !== "image") return null;
  // An external current file does not need a local snapshot, and must not use
  // the local database-backed protocol even when its transport is unavailable.
  if (source) return source.imagePreviewUrl?.(file.id, file.updatedAt, purpose) ?? null;
  if (file.currentVersion === null) return null;
  if (!isTauri()) return file.id.startsWith("fixture-file-") ? fixtureImageSource(file) : null;
  return convertFileSrc(file.id, "lumetrace-file-preview");
}

function FileArtwork({
  file,
  onImageDimensions,
  showVersionBadge = true,
}: {
  file: FileSpaceFileRecord;
  onImageDimensions?: (fileId: string, fileUpdatedAt: number, width: number, height: number) => void;
  showVersionBadge?: boolean;
}) {
  const { t } = useTranslation();
  const Icon = fileIcon(file);
  const artworkFormat = fileArtworkFormat(file);
  const archiveExtension = fileArchiveExtension(file);
  const artworkTitle = fileArtworkTitle(file);
  const source = useWorkspaceExtension()?.source ?? null;
  const previewSource = filePreviewSource(file, source);
  const store = source?.imagePreviews;
  const subscribe = useCallback((listener: () => void) => store?.subscribe(listener) ?? (() => {}), [store]);
  const snapshot = useCallback(() => previewSource ? store?.snapshot(previewSource) ?? null : null, [store, previewSource]);
  const retained = useSyncExternalStore(subscribe, snapshot, snapshot);
  useEffect(() => {
    if (store && previewSource) return store.retain(previewSource);
  }, [store, previewSource]);
  const previewKey = JSON.stringify([file.id, file.updatedAt, previewSource]);
  const [preview, setPreview] = useState<{ key: string; state: "loading" | "loaded" | "failed" }>({ key: previewKey, state: "loading" });
  // Reset before committing a changed image, so cached load events cannot be
  // overwritten by a later reset effect for the previous source/version.
  if (preview.key !== previewKey) setPreview({ key: previewKey, state: "loading" });
  const previewState = store ? (retained?.failed ? "failed" : retained?.url ? "loaded" : "loading")
    : preview.key === previewKey ? preview.state : "loading";
  const displaySource = store ? retained?.url : previewSource;
  const showPreview = Boolean(previewSource) && previewState !== "failed";
  const previewLoading = showPreview && previewState === "loading";
  return (
    <span aria-busy={previewLoading || undefined} className={`file-space-file-art is-${fileCategory(file)}${artworkFormat ? ` is-format-${artworkFormat}` : ""}${showPreview ? " has-preview" : ""}${previewLoading ? " is-preview-loading" : ""}`}>
      {showPreview && !displaySource ? null : showPreview ? (
        <img
          key={previewKey}
          src={displaySource ?? undefined}
          data-detail-source={source ? filePreviewSource(file, source, "detail") ?? undefined : undefined}
          data-preview-revision={source ? file.updatedAt : undefined}
          data-original-source={source?.imageOriginalUrl?.(file.id, file.updatedAt) ?? undefined}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={(event) => {
            setPreview({ key: previewKey, state: "loaded" });
            const { naturalWidth, naturalHeight } = event.currentTarget;
            if (naturalWidth > 0 && naturalHeight > 0) {
              onImageDimensions?.(file.id, file.updatedAt, naturalWidth, naturalHeight);
            }
          }}
          onError={() => {
            if (store && previewSource) store.fail(previewSource);
            setPreview({ key: previewKey, state: "failed" });
          }}
        />
      ) : artworkFormat ? (
        <span className={`file-space-format-art is-${artworkFormat}`}>
          {artworkFormat !== "archive" ? (
            <span className="file-space-format-kicker">
              {artworkFormat === "md" ? "MD" : artworkFormat.toUpperCase()}
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
      {showVersionBadge && shouldShowFileVersionBadge(file.versionCount) ? (
        <span
          className="file-space-file-version-count"
          title={t("fileSpace.content.versionCount", { count: file.versionCount })}
        >
          <Clock3 size={11} aria-hidden="true" />
          <span>{t("fileSpace.content.versionCount", { count: file.versionCount })}</span>
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

function FolderTreeChildren({
  expanded,
  renderChildren,
}: {
  expanded: boolean;
  renderChildren: () => React.ReactNode;
}) {
  const presence = usePresence(expanded, 140);
  if (!presence.mounted) return null;
  return (
    <div
      className="file-space-tree-children"
      data-state={presence.state}
      aria-hidden={presence.state === "closed"}
    >
      <div>{renderChildren()}</div>
    </div>
  );
}

export function FileSpacePage() {
  const invoke = useWorkspaceInvoke();
  const applicationExtension = useApplicationExtension();
  const externalWorkspace = useWorkspaceExtension();
  const capabilities = externalWorkspace?.source?.capabilities;
  const canWrite = capabilities?.write !== false;
  const canSearch = capabilities?.search !== false;
  const currentFolderStorageKey = externalWorkspace
    ? `${currentFolderStorageKeyBase}:${externalWorkspace.selectionKey}` : currentFolderStorageKeyBase;
  const { t, i18n } = useTranslation();
  const { appearance } = useTheme();
  const [snapshot, setSnapshot] = useState<FileSpaceSnapshot>(emptySnapshot);
  const [workspaceDirectory, setWorkspaceDirectory] = useState<FileSpaceWorkspaceDirectory | null>(null);
  const [workspaceGeneration, setWorkspaceGeneration] = useState(0);
  const [loading, setLoading] = useState(true);
  const [initialLoadError, setInitialLoadError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<FileSpaceBusyAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [setupCancelled, setSetupCancelled] = useState(false);
  const [rootSetupMode, setRootSetupMode] = useState<RootSetupMode | null>(null);
  const [currentFolderId, setCurrentFolderId] = useState<string | null>(() => (
    window.localStorage.getItem(currentFolderStorageKey)
  ));
  const [activeCollection, setActiveCollection] = useState<FileSpaceCollection>("files");
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [isGlobalSearchOpen, setGlobalSearchOpen] = useState(false);
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [searchScopes, setSearchScopes] = useState<SearchScope[]>(["name", "content", "tag"]);
  const [searchResult, setSearchResult] = useState<{ key: string; ids: Set<string> } | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [tagFilter, setTagFilter] = useState("all");
  const [timeFilter, setTimeFilter] = useState<TimeFilter>("all");
  const [sortOption, setSortOption] = useState<SortOption>(storedSortOption);
  const [fileLayoutMode, setFileLayoutMode] = useState<FileLayoutMode>(storedFileLayoutMode);
  const [previewSize, setPreviewSize] = useState(storedPreviewSize);
  const [fileJustifiedLayout, setFileJustifiedLayout] = useState<JustifiedFileLayout | null>(null);
  const [filePageCursor, setFilePageCursor] = useState<FileSpaceFilePageCursor | null>(null);
  const [filePageTotal, setFilePageTotal] = useState(0);
  const [filePageLoading, setFilePageLoading] = useState(false);
  const [filePageRevision, setFilePageRevision] = useState(0);
  const [fileRevealRevision, setFileRevealRevision] = useState(0);
  const [fileVirtualViewport, setFileVirtualViewport] = useState({ top: 0, bottom: 2_000 });
  const [imageDimensionsByFileVersion, setImageDimensionsByFileVersion] = useState<Record<string, FileImageDimensions>>({});
  const [isFilterMenuOpen, setFilterMenuOpen] = useState(false);
  const [isCreateFolderOpen, setCreateFolderOpen] = useState(false);
  const [createParentId, setCreateParentId] = useState<string | null>(null);
  const [folderName, setFolderName] = useState("");
  const [renameFolderId, setRenameFolderId] = useState<string | null>(null);
  const [renameFolderName, setRenameFolderName] = useState("");
  const [deleteFolderId, setDeleteFolderId] = useState<string | null>(null);
  const [folderContextMenu, setFolderContextMenu] = useState<FolderContextMenu | null>(null);
  const [fileContextMenu, setFileContextMenu] = useState<FileContextMenu | null>(null);
  const [contentContextMenu, setContentContextMenu] = useState<ContentContextMenu | null>(null);
  const [createFileFormat, setCreateFileFormat] = useState<NewTextFileFormat | null>(null);
  const [createFileParentId, setCreateFileParentId] = useState<string | null>(null);
  const [createFileName, setCreateFileName] = useState("");
  const [renameFileId, setRenameFileId] = useState<string | null>(null);
  const [renameFileName, setRenameFileName] = useState("");
  const [deleteFileId, setDeleteFileId] = useState<string | null>(null);
  const [tagFileId, setTagFileId] = useState<string | null>(null);
  const [tagDraft, setTagDraft] = useState("");
  const [timeline, setTimeline] = useState<TaskFileTimelineRecord | null>(null);
  useVersionAnnotationUpdates(setTimeline);
  const [timelineFile, setTimelineFile] = useState<FileSpaceFileRecord | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [versionPreview, setVersionPreview] = useState<string | null>(null);
  const [timelineBusy, setTimelineBusy] = useState(false);
  const [isTimelinePanelOpen, setTimelinePanelOpen] = useState(false);
  const [openingTimelineFileId, setOpeningTimelineFileId] = useState<string | null>(null);
  const [diffBeforeVersionId, setDiffBeforeVersionId] = useState<string | null>(null);
  const [diffAfterVersionId, setDiffAfterVersionId] = useState<string | null>(null);
  const [versionDiffStatus, setVersionDiffStatus] = useState<VersionDiffStatus>("idle");
  const [versionDiffResult, setVersionDiffResult] = useState<VersionDiffResult | null>(null);
  const [versionDiffError, setVersionDiffError] = useState<string | null>(null);
  const [versionDiffRetryToken, setVersionDiffRetryToken] = useState(0);
  const [isFileDragOver, setFileDragOver] = useState(false);
  const [internalFileDrag, setInternalFileDrag] = useState<InternalFileDrag | null>(null);
  const [internalFolderDrag, setInternalFolderDrag] = useState<InternalFolderDrag | null>(null);
  const [importFeedback, setImportFeedback] = useState<FileSpaceImportFeedback | null>(null);
  const [fileMoveFeedback, setFileMoveFeedback] = useState<FileMoveFeedback | null>(null);
  const [trashFeedback, setTrashFeedback] = useState<string | null>(null);
  const [importConflictFeedback, setImportConflictFeedback] = useState<IdenticalImportNotice | null>(null);
  const [versionNotifications, setVersionNotifications] = useState<FileSpaceVersionNotification[]>([]);
  const versionNotification = versionNotifications[0] ?? null;
  const [firstVersionNotificationId, setFirstVersionNotificationId] = useState<string | null>(null);
  const [restoreConfirmationEntryId, setRestoreConfirmationEntryId] = useState<string | null>(null);
  const [isEmptyTrashConfirmationOpen, setEmptyTrashConfirmationOpen] = useState(false);
  const [emptyTrashEntryIds, setEmptyTrashEntryIds] = useState<string[]>([]);
  const [restoringTrashEntryId, setRestoringTrashEntryId] = useState<string | null>(null);
  const [selectedFileIds, setSelectedFileIds] = useState<Set<string>>(new Set());
  const [selectionMarquee, setSelectionMarquee] = useState<FileSelectionMarquee | null>(null);
  const [selectionAnnouncement, setSelectionAnnouncement] = useState("");
  const [inspectorTimeline, setInspectorTimeline] = useState<TaskFileTimelineRecord | null>(null);
  const [inspectorTimelineLoading, setInspectorTimelineLoading] = useState(false);
  const [isSidebarVisible, setSidebarVisible] = useState(() => window.matchMedia("(min-width: 981px)").matches);
  const [isInspectorVisible, setInspectorVisible] = useState(() => (
    storedInspectorVisibility(window.localStorage)
  ));
  const [pendingImportPath, setPendingImportPath] = useState<string | null>(null);
  const [pendingFileImportConflict, setPendingFileImportConflict] = useState<PendingFileImportConflict | null>(null);
  const recordImageDimensions = useCallback((fileId: string, fileUpdatedAt: number, width: number, height: number) => {
    setImageDimensionsByFileVersion((current) => {
      const key = imageDimensionsKey(fileId, fileUpdatedAt);
      const previous = current[key];
      if (
        previous?.fileUpdatedAt === fileUpdatedAt
        && previous.width === width
        && previous.height === height
      ) return current;
      return { ...current, [key]: { fileUpdatedAt, width, height } };
    });
  }, []);
  const folderNameRef = useRef<HTMLInputElement>(null);
  const createFileNameRef = useRef<HTMLInputElement>(null);
  const renameFolderNameRef = useRef<HTMLInputElement>(null);
  const renameFileNameRef = useRef<HTMLInputElement>(null);
  const tagDraftRef = useRef<HTMLInputElement>(null);
  const deleteFolderCancelRef = useRef<HTMLButtonElement>(null);
  const deleteFileCancelRef = useRef<HTMLButtonElement>(null);
  const restoreTrashCancelRef = useRef<HTMLButtonElement>(null);
  const emptyTrashCancelRef = useRef<HTMLButtonElement>(null);
  const importConflictCancelRef = useRef<HTMLButtonElement>(null);
  const dialogReturnFocusRef = useRef<HTMLElement | null>(null);
  const dialogFallbackFocusRef = useRef<HTMLElement | null>(null);
  const dialogWasOpenRef = useRef(false);
  const dialogRestorePendingRef = useRef(false);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const contextMenuReturnFocusRef = useRef<HTMLElement | null>(null);
  const filterMenuRef = useRef<HTMLDivElement>(null);
  const contentDropZoneRef = useRef<HTMLDivElement>(null);
  const fileGridRef = useRef<HTMLDivElement>(null);
  const sidebarRef = useRef<HTMLElement>(null);
  const sidebarScrollRef = useRef<HTMLDivElement>(null);
  const folderTreeRef = useRef<HTMLDivElement>(null);
  const timelinePanelRef = useRef<HTMLElement>(null);
  const timelineBadgeReturnFocusRef = useRef<HTMLElement | null>(null);
  const importExistingTriggerRef = useRef<HTMLElement | null>(null);
  const operationRef = useRef<FileSpaceOperation | null>(null);
  const lifecycleRef = useRef({ mounted: false, generation: 0 });
  const nativeDropBlockedRef = useRef(false);
  const externalDragDepthRef = useRef(0);
  const searchSequenceRef = useRef(0);
  const filePageSequenceRef = useRef(0);
  const filePageLoadingRef = useRef(false);
  const measuredFileLayoutRef = useRef<JustifiedFileLayout | null>(null);
  const fileVirtualViewportFrameRef = useRef<number | null>(null);
  const fileRevealNavigationRef = useRef(new FileRevealNavigation<FileSpaceSearchFile>());
  const aiSourceNavigationSequenceRef = useRef(0);
  const importRequestRef = useRef<string | null>(null);
  const autoTrashCleanupRetryAtRef = useRef(0);
  const autoTrashCleanupErrorRef = useRef<string | null>(null);
  const timelineRequestRef = useRef(0);
  const timelineMutationRequestRef = useRef(0);
  const versionPreviewRequestRef = useRef(0);
  const versionDiffRequestRef = useRef(0);
  const fileDragGestureRef = useRef<FileDragGesture | null>(null);
  const internalFileDragRef = useRef<InternalFileDrag | null>(null);
  const selectedFileIdsRef = useRef<Set<string>>(new Set());
  const selectionAnchorRef = useRef<string | null>(null);
  const pointerSelectionIntentRef = useRef<PointerSelectionIntent | null>(null);
  const suppressNextFileClickRef = useRef(false);
  const marqueeGestureRef = useRef<FileMarqueeGesture | null>(null);
  const marqueeAutoScrollFrameRef = useRef<number | null>(null);
  const folderDragGestureRef = useRef<FolderDragGesture | null>(null);
  const internalFolderDragRef = useRef<InternalFolderDrag | null>(null);
  const suppressFolderClickRef = useRef(false);
  const folderClickReleaseTimerRef = useRef<number | null>(null);
  const fileDragStartingRef = useRef(false);
  const fileDragReleaseTimerRef = useRef<number | null>(null);
  const fileDragCancelTimerRef = useRef<number | null>(null);
  const createDialogPresence = usePresence(isCreateFolderOpen);
  const createFileDialogPresence = usePresence(Boolean(createFileFormat));
  const renameDialogPresence = usePresence(Boolean(renameFolderId));
  const deleteDialogPresence = usePresence(Boolean(deleteFolderId));
  const renameFileDialogPresence = usePresence(Boolean(renameFileId));
  const deleteFileDialogPresence = usePresence(Boolean(deleteFileId));
  const restoreTrashDialogPresence = usePresence(Boolean(restoreConfirmationEntryId));
  const emptyTrashDialogPresence = usePresence(isEmptyTrashConfirmationOpen);
  const tagDialogPresence = usePresence(Boolean(tagFileId));
  const importConflictDialogPresence = usePresence(Boolean(pendingFileImportConflict));
  const filterMenuPresence = usePresence(isFilterMenuOpen, 140);
  const timelinePanelPresence = usePresence(isTimelinePanelOpen, 140);
  const isFileSpaceDialogOpen = Boolean(
    isCreateFolderOpen
    || createFileFormat
    || renameFolderId
    || deleteFolderId
    || renameFileId
    || deleteFileId
    || restoreConfirmationEntryId
    || isEmptyTrashConfirmationOpen
    || tagFileId
    || pendingFileImportConflict
  );
  const isFileSpaceDialogMounted = Boolean(
    createDialogPresence.mounted
    || createFileDialogPresence.mounted
    || renameDialogPresence.mounted
    || deleteDialogPresence.mounted
    || renameFileDialogPresence.mounted
    || deleteFileDialogPresence.mounted
    || restoreTrashDialogPresence.mounted
    || emptyTrashDialogPresence.mounted
    || tagDialogPresence.mounted
    || importConflictDialogPresence.mounted
  );

  const locale = i18n.resolvedLanguage ?? "en-US";
  const fileListDateFormatter = useMemo(() => new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }), [locale]);
  const searchKey = `${query.trim()}\u0000${[...searchScopes].sort().join(",")}`;
  const globalSearchShortcut = globalSearchShortcutLabel();
  const fileDetailsHeight = Math.max(
    fileDetailsHeightFallback,
    appearance.bodyFontSize * 2 + 12,
  );

  const closeTimelinePanel = useCallback(() => {
    timelineRequestRef.current += 1;
    versionPreviewRequestRef.current += 1;
    versionDiffRequestRef.current += 1;
    setTimelineBusy(false);
    setOpeningTimelineFileId(null);
    setTimelinePanelOpen(false);
    const trigger = timelineBadgeReturnFocusRef.current;
    timelineBadgeReturnFocusRef.current = null;
    if (trigger?.isConnected) window.requestAnimationFrame(() => trigger.focus({ preventScroll: true }));
  }, []);

  const openGlobalSearch = useCallback(() => {
    if (!canSearch) return;
    const activeModal = document.querySelector<HTMLElement>('[aria-modal="true"]');
    if (activeModal && !activeModal.classList.contains("file-space-global-search-panel")) return;
    setFilterMenuOpen(false);
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setGlobalSearchOpen(true);
  }, []);

  useEffect(() => {
    if (!isTimelinePanelOpen || !timelinePanelPresence.mounted) return undefined;
    const panel = timelinePanelRef.current;
    if (!panel) return undefined;
    const focusCloseButton = () => panel.querySelector<HTMLButtonElement>("header > button")?.focus({ preventScroll: true });
    const frame = window.requestAnimationFrame(focusCloseButton);
    const handleTimelineKeyDown = (event: KeyboardEvent) => {
      // Portalled version menus handle their own Escape, arrows and Tab first.
      if (event.defaultPrevented) return;
      if (event.target instanceof Element && event.target.closest(".mac-select-menu")) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        // The menu can be open before its next-frame focus transfer finishes.
        const expandedSelect = panel.querySelector<HTMLButtonElement>('.mac-select-trigger[aria-expanded="true"]');
        if (expandedSelect) {
          expandedSelect.click();
          expandedSelect.focus({ preventScroll: true });
          return;
        }
        closeTimelinePanel();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(panel.querySelectorAll<HTMLElement>(
        'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
      )).filter((element) => element.getClientRects().length > 0);
      const first = focusable[0] ?? panel;
      const last = focusable[focusable.length - 1] ?? panel;
      const active = document.activeElement;
      if (!panel.contains(active) || (event.shiftKey ? active === first : active === last)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
    };
    const keepTimelineFocus = (event: FocusEvent) => {
      const target = event.target;
      if (target instanceof Element && target.closest(".file-version-note-dialog")) return;
      if (target instanceof Element && target.closest(".mac-select-menu")) return;
      if (!panel.contains(target as Node)) focusCloseButton();
    };
    window.addEventListener("keydown", handleTimelineKeyDown);
    window.addEventListener("focusin", keepTimelineFocus);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", handleTimelineKeyDown);
      window.removeEventListener("focusin", keepTimelineFocus);
    };
  }, [closeTimelinePanel, isTimelinePanelOpen, timelinePanelPresence.mounted]);

  useEffect(() => {
    if (timelinePanelPresence.mounted) return;
    setTimeline(null);
    setTimelineFile(null);
    setSelectedVersionId(null);
    setVersionPreview(null);
    setDiffBeforeVersionId(null);
    setDiffAfterVersionId(null);
    setVersionDiffStatus("idle");
    setVersionDiffResult(null);
    setVersionDiffError(null);
  }, [timelinePanelPresence.mounted]);

  useEffect(() => {
    if (
      !isTimelinePanelOpen
      || !timeline
      || !timelineFile
      || !diffBeforeVersionId
      || !diffAfterVersionId
    ) {
      setVersionDiffStatus("idle");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      return undefined;
    }

    const beforeVersion = timeline.versions.find((version) => version.id === diffBeforeVersionId);
    const afterVersion = timeline.versions.find((version) => version.id === diffAfterVersionId);
    if (!beforeVersion || !afterVersion) {
      setVersionDiffStatus("idle");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      return undefined;
    }
    if (!supportsTextPreview(beforeVersion) || !supportsTextPreview(afterVersion)) {
      setVersionDiffStatus("unsupported");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      return undefined;
    }
    if (beforeVersion.sizeBytes > maxVersionDiffTextBytes || afterVersion.sizeBytes > maxVersionDiffTextBytes) {
      setVersionDiffStatus("tooLarge");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      return undefined;
    }

    const requestId = versionDiffRequestRef.current + 1;
    versionDiffRequestRef.current = requestId;
    setVersionDiffStatus("loading");
    setVersionDiffResult(null);
    setVersionDiffError(null);

    const readVersionText = async (version: TaskFileVersionRecord) => {
      if (!isTauri()) return createVisualVersionContent(timelineFile, version);
      const bytes = await invoke<number[]>("read_task_file_version", {
        fileId: timeline.fileId,
        versionId: version.id,
      });
      return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
    };

    void (async () => {
      try {
        const [beforeText, afterText] = await Promise.all([
          readVersionText(beforeVersion),
          readVersionText(afterVersion),
        ]);
        if (requestId !== versionDiffRequestRef.current || !lifecycleRef.current.mounted) return;
        setVersionDiffResult(diffTextVersions(beforeText, afterText));
        setVersionDiffStatus("ready");
      } catch (diffError) {
        if (requestId !== versionDiffRequestRef.current || !lifecycleRef.current.mounted) return;
        setVersionDiffResult(null);
        setVersionDiffError(errorText(diffError));
        setVersionDiffStatus("error");
      }
    })();

    return () => {
      if (versionDiffRequestRef.current === requestId) versionDiffRequestRef.current += 1;
    };
  }, [
    diffAfterVersionId,
    diffBeforeVersionId,
    isTimelinePanelOpen,
    timeline,
    timelineFile,
    versionDiffRetryToken,
  ]);

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
          const matches = isTauri()
            ? await invoke<FileSpaceSearchMatch[]>("search_file_space_files", {
                request: { query: normalized, scopes: searchScopes },
              })
            : snapshot.files
                .filter((file) => (
                  (searchScopes.includes("name") && matchesSearch(file.name, normalized))
                  || (searchScopes.includes("tag") && file.tags.some((tag) => matchesSearch(tag, normalized)))
                ))
                .map((file) => ({
                  fileId: file.id,
                  lexicalMatch: true,
                  semanticSimilarity: null,
                }));
          if (sequence !== searchSequenceRef.current) return;
          setSearchResult({
            key: searchKey,
            ids: new Set(matches.slice(0, 500).map((match) => match.fileId)),
          });
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
  }, [query, searchKey, searchScopes, t]);

  const updatePreviewSize = (value: number) => {
    const nextSize = clampPreviewSize(value);
    setPreviewSize(nextSize);
    window.localStorage.setItem(previewSizeStorageKey, String(nextSize));
  };

  const updateSortOption = (value: SortOption) => {
    setSortOption(value);
    window.localStorage.setItem(sortOptionStorageKey, value);
  };

  const updateFileLayoutMode = (value: FileLayoutMode) => {
    setFileLayoutMode(value);
    window.localStorage.setItem(fileLayoutModeStorageKey, value);
  };

  const toggleSidebarVisibility = () => {
    setSidebarVisible((visible) => !visible);
    setContentContextMenu(null);
  };

  const toggleInspectorVisibility = () => {
    setInspectorVisible((visible) => !visible);
    setContentContextMenu(null);
  };

  const rememberDialogReturnFocus = (fallback: HTMLElement | null = null) => {
    dialogReturnFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    dialogFallbackFocusRef.current = fallback;
    dialogRestorePendingRef.current = false;
  };

  const updateInternalFileDrag = (value: InternalFileDrag | null) => {
    internalFileDragRef.current = value;
    setInternalFileDrag(value);
  };

  const updateSelectedFiles = (value: Set<string>, anchorId?: string | null) => {
    if (anchorId !== undefined) selectionAnchorRef.current = anchorId;
    if (setsEqual(value, selectedFileIdsRef.current)) return;
    selectedFileIdsRef.current = value;
    setSelectedFileIds(value);
  };

  const clearSelectedFiles = () => {
    updateSelectedFiles(new Set(), null);
    setInspectorTimeline(null);
  };

  const revealFileInWorkspace = (file: FileSpaceSearchFile, openAfterReveal = false) => {
    fileRevealNavigationRef.current.start(file, openAfterReveal, !isTauri());
    setGlobalSearchOpen(false);
    setActiveCollection("files");
    setCurrentFolderId(file.folderId);
    setQuery("");
    setSearchResult(null);
    setTypeFilter("all");
    setTagFilter("all");
    setTimeFilter("all");
    setFilterMenuOpen(false);
    setInspectorVisible(true);
    updateSelectedFiles(new Set([file.id]), file.id);
    setFileRevealRevision((revision) => revision + 1);
    setFilePageRevision((revision) => revision + 1);
    if (file.folderId) {
      setExpandedFolders((current) => {
        const next = new Set(current);
        let folderId: string | null = file.folderId;
        while (folderId) {
          next.add(folderId);
          folderId = snapshot.folders.find((folder) => folder.id === folderId)?.parentId ?? null;
        }
        return next;
      });
    }
  };

  const openFileFromGlobalSearch = (file: FileSpaceSearchFile) => {
    revealFileInWorkspace(file);
  };

  const openFileFromAiSource = async (
    source: FileSpaceAiSourceReference,
    action: "select" | "open",
  ) => {
    const generation = lifecycleRef.current.generation;
    const sequence = aiSourceNavigationSequenceRef.current + 1;
    aiSourceNavigationSequenceRef.current = sequence;
    fileRevealNavigationRef.current.cancel();
    let file = snapshot.files.find((candidate) => candidate.id === source.fileId);
    if (!file && isTauri()) {
      try {
        const page = await invoke<FileSpaceFilePage>("list_file_space_files", {
          request: {
            folderId: null,
            sort: "updatedDesc",
            typeFilter: "all",
            tagFilter: null,
            updatedAfter: null,
            matchIds: [source.fileId],
            cursor: null,
            limit: 1,
          },
        });
        [file] = page.files;
      } catch {
        file = undefined;
      }
    }
    if (
      sequence !== aiSourceNavigationSequenceRef.current
      || !lifecycleRef.current.mounted
      || generation !== lifecycleRef.current.generation
    ) return;
    if (file) revealFileInWorkspace(file, action === "open");
    else setError(t("fileSpace.ai.errors.sourceUnavailable"));
  };

  const updateInternalFolderDrag = (value: InternalFolderDrag | null) => {
    internalFolderDragRef.current = value;
    setInternalFolderDrag(value);
  };

  const beginOperation = (action: FileSpaceBusyAction) => {
    if (!canWrite && action !== "configure") return null;
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
    setInitialLoadError(null);
    setError(null);
    try {
      const [loaded, loadedWorkspaceDirectory] = await Promise.all([
        invoke<FileSpaceSnapshot>("get_file_space_snapshot"),
        invoke<FileSpaceWorkspaceDirectory>("get_file_space_workspaces").then(directory => {
          if (lifecycleRef.current.mounted && lifecycleRef.current.generation === generation) setWorkspaceDirectory(directory);
          return directory;
        }),
      ]);
      if (
        !lifecycleRef.current.mounted ||
        lifecycleRef.current.generation !== generation
      ) return;
      setSnapshot(loaded);
      setFilePageTotal(loaded.fileCount);
      setFilePageCursor(null);
      setWorkspaceDirectory(loadedWorkspaceDirectory);
      setExpandedFolders(new Set());
      setCurrentFolderId((current) => (
        current && loaded.folders.some((folder) => folder.id === current) ? current : null
      ));
    } catch (loadError) {
      if (
        lifecycleRef.current.mounted &&
        lifecycleRef.current.generation === generation
      ) {
        setInitialLoadError(errorText(loadError));
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
      aiSourceNavigationSequenceRef.current += 1;
      fileRevealNavigationRef.current.cancel();
      if (fileDragReleaseTimerRef.current !== null) {
        window.clearTimeout(fileDragReleaseTimerRef.current);
        fileDragReleaseTimerRef.current = null;
      }
      if (fileDragCancelTimerRef.current !== null) {
        window.clearTimeout(fileDragCancelTimerRef.current);
        fileDragCancelTimerRef.current = null;
      }
      if (fileVirtualViewportFrameRef.current !== null) {
        window.cancelAnimationFrame(fileVirtualViewportFrameRef.current);
        fileVirtualViewportFrameRef.current = null;
      }
      fileDragStartingRef.current = false;
      fileDragGestureRef.current = null;
      internalFileDragRef.current = null;
      marqueeGestureRef.current = null;
      if (marqueeAutoScrollFrameRef.current !== null) {
        window.cancelAnimationFrame(marqueeAutoScrollFrameRef.current);
        marqueeAutoScrollFrameRef.current = null;
      }
      folderDragGestureRef.current = null;
      internalFolderDragRef.current = null;
      timelineRequestRef.current += 1;
      versionPreviewRequestRef.current += 1;
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
    const narrowWindow = window.matchMedia("(max-width: 980px)");
    const collapseSidebar = (event: MediaQueryListEvent) => {
      if (event.matches) setSidebarVisible(false);
    };
    narrowWindow.addEventListener("change", collapseSidebar);
    return () => narrowWindow.removeEventListener("change", collapseSidebar);
  }, []);

  useEffect(() => {
    if (!isSidebarVisible || !window.matchMedia("(max-width: 980px)").matches) return undefined;
    const closeSidebarOutside = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest("[data-file-space-sidebar-toggle]")) return;
      if (!sidebarRef.current?.contains(event.target as Node)) setSidebarVisible(false);
    };
    window.addEventListener("pointerdown", closeSidebarOutside);
    return () => window.removeEventListener("pointerdown", closeSidebarOutside);
  }, [isSidebarVisible]);

  useEffect(() => {
    if (!isTauri()) {
      if (import.meta.env.DEV) {
        const previewParameters = new URLSearchParams(window.location.search);
        if (previewParameters.has("setupPreview")) {
          const setupPreview = previewParameters.get("setupPreview");
          setSnapshot({
            ...emptySnapshot,
            rootPath: setupPreview === "missing" ? "/Volumes/Design Archive/Lume Trace" : null,
            rootStatus: setupPreview === "missing" ? "missing" : "unconfigured",
          });
          if (setupPreview === "busy") {
            setBusyAction("configure");
            setRootSetupMode("new");
          }
        } else if (
          previewParameters.has("fileSpacePreview")
          || previewParameters.has("trashPreview")
          || previewParameters.has("emptyPreview")
          || previewParameters.has("timelinePreview")
        ) {
          const fixture = createVisualFixture();
          if (previewParameters.has("fileSpacePreview") && previewParameters.has("previewVersionCount")) {
            // Synthetic preview metadata only; never reads or changes a real file.
            const count = Number(previewParameters.get("previewVersionCount"));
            if (Number.isInteger(count) && count >= 0 && count <= 40) {
              fixture.files[7] = {
                ...fixture.files[7],
                name: previewParameters.has("previewLongName")
                  ? "Launch Plan with a very long filename for checking version count and toolbar layout at minimum window size.md"
                  : "Launch Plan.md",
                versionCount: count,
                currentVersion: count || null,
              };
            }
          }
          if (previewParameters.has("timelinePreview")) {
            // Browser-only density fixture: no workspace, snapshot, or service access.
            const file: FileSpaceFileRecord = {
              ...fixture.files[7],
              id: "fixture-timeline-scroll",
              name: "Synthetic version history with a long filename for scroll verification.md",
              mimeType: "text/markdown",
              versionCount: 40,
              currentVersion: 40,
              sizeBytes: 32_000,
            };
            fixture.files[7] = file;
            const previewTimeline = createVisualTimeline(file)!;
            previewTimeline.events = previewTimeline.versions.map((version) => ({
              id: `fixture-event-${version.id}`,
              versionId: version.id,
              eventType: "current_version_changed",
              actor: "user",
              details: {},
              createdAt: version.producedAt,
            }));
            const comparison = defaultVersionComparison(previewTimeline.versions, previewTimeline.currentVersionId);
            setTimelineFile(file);
            setTimeline(previewTimeline);
            setSelectedVersionId(previewTimeline.currentVersionId);
            setVersionPreview(createVisualVersionContent(file, previewTimeline.versions[0]));
            setDiffBeforeVersionId(comparison?.beforeVersionId ?? null);
            setDiffAfterVersionId(comparison?.afterVersionId ?? null);
            setTimelinePanelOpen(true);
          }
          if (previewParameters.get("trashPreview") === "empty") {
            fixture.trashItems = [];
          }
          const emptyPreview = previewParameters.get("emptyPreview");
          if (emptyPreview === "folder") {
            fixture.folders.push({
              id: "fixture-empty",
              parentId: "fixture-research",
              name: "待整理",
              relativePath: "Clipboard X/市场调研/待整理",
              manualOrder: 2,
              directFileCount: 0,
              fileCount: 0,
              childFolderCount: 0,
              createdAt: Date.now(),
              updatedAt: Date.now(),
            });
          }
          if (emptyPreview === "filter") {
            fixture.files = fixture.files.map((file) => ({ ...file, folderId: "fixture-comments" }));
          }
          setSnapshot(fixture);
          setFilePageTotal(emptyPreview ? 0 : fixture.files.length);
          setWorkspaceDirectory({
            currentWorkspaceId: "fixture-workspace",
            workspaces: [{
              id: "fixture-workspace",
              name: fixture.rootName ?? "Lume Trace",
              kind: "local",
              rootPath: fixture.rootPath,
              memberCount: 0,
              current: true,
              createdAt: Date.now(),
              lastOpenedAt: Date.now(),
            }],
          });
          setExpandedFolders(new Set(fixture.folders.map((folder) => folder.id)));
          setCurrentFolderId(
            emptyPreview === "folder"
              ? "fixture-empty"
              : emptyPreview === "filter"
                ? "fixture-comments"
                : "fixture-research",
          );
          if (emptyPreview === "search") setQuery("找不到的文件");
          if (emptyPreview === "filter") setTagFilter("未使用的标签");
          if (previewParameters.has("trashPreview")) setActiveCollection("trash");
          if (previewParameters.has("versionNotificationPreview")) {
            const file = fixture.files[0];
            if (file) {
              setVersionNotifications([{
                versionId: "fixture-external-version",
                fileId: file.id,
                fileName: file.name,
                versionNumber: Math.max(2, file.versionCount + 1),
                createdAt: Date.now(),
              }]);
            }
          }
          if (previewParameters.has("importConflictPreview")) {
            const count = previewParameters.get("importConflictPreview") === "multiple" ? 3 : 1;
            const conflicts = fixture.files.slice(0, count).map((file, index) => ({
              relativePath: file.name,
              fileName: index === 0 ? "常用.md" : file.name,
              existingFileId: file.id,
              existingVersion: Math.max(1, file.currentVersion ?? 1),
              existingSizeBytes: file.sizeBytes,
              incomingSizeBytes: file.sizeBytes + ((index + 1) * 1_024),
              identical: false,
            }));
            setPendingFileImportConflict({
              requestId: "fixture-import-conflict",
              folderId: "fixture-research",
              source: {
                kind: "droppedFiles",
                files: conflicts.map((conflict) => ({
                  relativePath: conflict.relativePath,
                  bytes: [],
                })),
              },
              conflicts,
              identicalFiles: [],
            });
          }
          if (previewParameters.has("identicalImportPreview")) {
            const file = fixture.files[0];
            if (file) {
              setImportConflictFeedback({
                files: [{ fileId: file.id, fileName: file.name }],
              });
            }
          }
        }
        if (previewParameters.has("importSheetPreview")) {
          setPendingImportPath("/Volumes/NAS Studio/产品资料");
        }
        if (previewParameters.get("setupPreview") === "loading") return;
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
    window.localStorage.setItem(inspectorVisibilityStorageKey, String(isInspectorVisible));
  }, [isInspectorVisible]);

  useEffect(() => {
    const applySavedMarkdownSnapshot = (event: Event) => {
      const next = (event as CustomEvent<FileSpaceSnapshot>).detail;
      if (next) {
        setSnapshot(next);
        // A successfully delivered snapshot resolves an earlier load failure.
        setInitialLoadError(null);
        if (isTauri()) {
          void invoke<FileSpaceWorkspaceDirectory>("get_file_space_workspaces")
            .then(setWorkspaceDirectory)
            .catch(() => undefined);
        }
      }
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
    if (!isTauri()) return undefined;
    let disposed = false;
    let stop: (() => void) | undefined;
    let scanInFlight = false;
    let drainInFlight = false;
    let drainAgain = false;

    const applyNotifications = async (notifications: FileSpaceVersionNotification[]) => {
      if (disposed || notifications.length === 0) return;
      setVersionNotifications((current) => mergeVersionNotifications(current, notifications));
      try {
        const refreshed = await invoke<FileSpaceSnapshot>("get_file_space_snapshot");
        if (!disposed) setSnapshot(refreshed);
      } catch {
        // The version notification remains actionable even if this visual refresh is delayed.
      }
    };

    const drainNotifications = async () => {
      if (drainInFlight) {
        drainAgain = true;
        return;
      }
      drainInFlight = true;
      try {
        do {
          drainAgain = false;
          const queued = await invoke<FileSpaceVersionNotification[]>(
            "drain_file_space_version_notifications",
          );
          await applyNotifications(queued);
        } while (!disposed && drainAgain);
      } catch {
        // A focus scan below provides a second chance to discover the same content change.
      } finally {
        drainInFlight = false;
      }
    };

    const scanExternalChanges = async () => {
      if (disposed || scanInFlight) return;
      scanInFlight = true;
      try {
        const discovered = await invoke<FileSpaceVersionNotification[]>(
          "scan_file_space_external_changes",
        );
        await applyNotifications(discovered);
      } catch {
        // Missing or disconnected workspaces are already represented by the main snapshot state.
      } finally {
        scanInFlight = false;
      }
    };

    const handleWindowFocus = () => void scanExternalChanges();
    window.addEventListener("focus", handleWindowFocus);
    void listen<FileSpaceVersionNotification>("file-space-version-created", () => {
      void drainNotifications();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      stop = unlisten;
      void drainNotifications().then(() => scanExternalChanges());
    });

    return () => {
      disposed = true;
      stop?.();
      window.removeEventListener("focus", handleWindowFocus);
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
    if (!trashFeedback) return undefined;
    const timer = window.setTimeout(() => setTrashFeedback(null), 2800);
    return () => window.clearTimeout(timer);
  }, [trashFeedback]);

  useEffect(() => {
    if (
      !isTauri()
      || snapshot.rootStatus !== "ready"
      || snapshot.trashItems.length === 0
    ) return undefined;
    let cancelled = false;
    let timer: number | null = null;
    const schedule = (delay: number) => {
      if (cancelled) return;
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => void runCleanup(), Math.min(delay, maxTrashCleanupTimerMs));
    };
    const runCleanup = async () => {
      timer = null;
      if (operationRef.current || dialogWasOpenRef.current) {
        schedule(1_000);
        return;
      }
      const operation = beginOperation("purgeTrash");
      if (!operation) {
        schedule(1_000);
        return;
      }
      try {
        const result = await invoke<FileSpaceTrashPurgeResult>("purge_expired_file_space_trash");
        if (!canCommitOperation(operation)) return;
        setSnapshot(result.snapshot);
        if (result.failedCount > 0 || result.failureMessage) {
          autoTrashCleanupRetryAtRef.current = Date.now() + 60_000;
          const summary = t("fileSpace.trash.partialPurge", {
            failed: result.failedCount,
            purged: result.purgedCount,
          });
          const message = `${t("fileSpace.errors.autoCleanupTrash")} ${summary}${result.failureMessage ? ` ${result.failureMessage}` : ""}`;
          autoTrashCleanupErrorRef.current = message;
          setError(message);
        } else {
          autoTrashCleanupRetryAtRef.current = 0;
          const previousCleanupError = autoTrashCleanupErrorRef.current;
          autoTrashCleanupErrorRef.current = null;
          if (previousCleanupError) {
            setError((current) => current === previousCleanupError ? null : current);
          }
          if (result.purgedCount > 0) {
            setTrashFeedback(t("fileSpace.trash.autoDeleted", { count: result.purgedCount }));
          }
        }
      } catch (cleanupError) {
        if (canCommitOperation(operation)) {
          autoTrashCleanupRetryAtRef.current = Date.now() + 60_000;
          const message = `${t("fileSpace.errors.autoCleanupTrash")} ${errorText(cleanupError)}`;
          autoTrashCleanupErrorRef.current = message;
          setError(message);
          schedule(60_000);
        }
      } finally {
        finishOperation(operation);
      }
    };
    const nextExpiration = Math.min(
      ...snapshot.trashItems.map((item) => item.trashedAt + trashRetentionMs),
    );
    const nextAttempt = Math.max(nextExpiration, autoTrashCleanupRetryAtRef.current);
    schedule(Math.max(0, nextAttempt - Date.now()));
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [snapshot.rootPath, snapshot.rootStatus, snapshot.trashItems, t]);

  useEffect(() => {
    if (!isCreateFolderOpen || !createDialogPresence.mounted) return;
    window.requestAnimationFrame(() => folderNameRef.current?.focus());
  }, [createDialogPresence.mounted, isCreateFolderOpen]);

  useEffect(() => {
    if (!renameFolderId || !renameDialogPresence.mounted) return;
    window.requestAnimationFrame(() => {
      renameFolderNameRef.current?.focus();
      renameFolderNameRef.current?.select();
    });
  }, [renameDialogPresence.mounted, renameFolderId]);

  useEffect(() => {
    if (!renameFileId || !renameFileDialogPresence.mounted) return;
    window.requestAnimationFrame(() => {
      renameFileNameRef.current?.focus();
      renameFileNameRef.current?.select();
    });
  }, [renameFileDialogPresence.mounted, renameFileId]);

  useEffect(() => {
    if (!createFileFormat || !createFileDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => {
      const input = createFileNameRef.current;
      if (!input) return;
      input.focus();
      input.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [createFileDialogPresence.mounted, createFileFormat]);

  useEffect(() => {
    if (!tagFileId || !tagDialogPresence.mounted) return;
    window.requestAnimationFrame(() => {
      tagDraftRef.current?.focus();
      tagDraftRef.current?.select();
    });
  }, [tagDialogPresence.mounted, tagFileId]);

  useEffect(() => {
    if (!deleteFolderId || !deleteDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => deleteFolderCancelRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [deleteDialogPresence.mounted, deleteFolderId]);

  useEffect(() => {
    if (!deleteFileId || !deleteFileDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => deleteFileCancelRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [deleteFileDialogPresence.mounted, deleteFileId]);

  useEffect(() => {
    if (!restoreConfirmationEntryId || !restoreTrashDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => restoreTrashCancelRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [restoreConfirmationEntryId, restoreTrashDialogPresence.mounted]);

  useEffect(() => {
    if (!isEmptyTrashConfirmationOpen || !emptyTrashDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => emptyTrashCancelRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [emptyTrashDialogPresence.mounted, isEmptyTrashConfirmationOpen]);

  useEffect(() => {
    if (!pendingFileImportConflict || !importConflictDialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => importConflictCancelRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [importConflictDialogPresence.mounted, pendingFileImportConflict]);

  useEffect(() => {
    if (isEmptyTrashConfirmationOpen || emptyTrashDialogPresence.mounted) return;
    setEmptyTrashEntryIds([]);
  }, [emptyTrashDialogPresence.mounted, isEmptyTrashConfirmationOpen]);

  useEffect(() => {
    if (isFileSpaceDialogOpen) {
      if (!dialogWasOpenRef.current) {
        dialogWasOpenRef.current = true;
        dialogRestorePendingRef.current = false;
        if (!dialogReturnFocusRef.current) {
          dialogReturnFocusRef.current = document.activeElement instanceof HTMLElement
            ? document.activeElement
            : null;
        }
      }
      return;
    }
    if (dialogWasOpenRef.current) {
      dialogWasOpenRef.current = false;
      dialogRestorePendingRef.current = true;
    }
    if (isFileSpaceDialogMounted || !dialogRestorePendingRef.current) return;
    dialogRestorePendingRef.current = false;
    const target = dialogReturnFocusRef.current?.isConnected
      ? dialogReturnFocusRef.current
      : dialogFallbackFocusRef.current?.isConnected
        ? dialogFallbackFocusRef.current
        : document.querySelector<HTMLElement>(".file-space-search-trigger");
    dialogReturnFocusRef.current = null;
    dialogFallbackFocusRef.current = null;
    if (target) window.requestAnimationFrame(() => target.focus());
  }, [isFileSpaceDialogMounted, isFileSpaceDialogOpen]);

  useEffect(() => {
    if (!isFileSpaceDialogOpen) return undefined;
    const trapDialogFocus = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const dialog = document.querySelector<HTMLElement>(".file-space-dialog-backdrop.is-open .file-space-dialog");
      if (!dialog) return;
      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), a[href]',
      )).filter((element) => element.offsetParent !== null);
      if (focusable.length === 0) {
        event.preventDefault();
        event.stopImmediatePropagation();
        dialog.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!dialog.contains(active)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        event.stopImmediatePropagation();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        event.stopImmediatePropagation();
        first.focus();
      }
    };
    window.addEventListener("keydown", trapDialogFocus, true);
    return () => window.removeEventListener("keydown", trapDialogFocus, true);
  }, [isFileSpaceDialogOpen]);

  useEffect(() => {
    if (
      !isCreateFolderOpen &&
      !createFileFormat &&
      !renameFolderId &&
      !deleteFolderId &&
      !renameFileId &&
      !deleteFileId &&
      !restoreConfirmationEntryId &&
      !isEmptyTrashConfirmationOpen &&
      !pendingFileImportConflict &&
      !tagFileId &&
      !isFilterMenuOpen &&
      !folderContextMenu &&
      !fileContextMenu &&
      !contentContextMenu &&
      !isTimelinePanelOpen
    ) return undefined;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const activeModal = document.querySelector<HTMLElement>('[aria-modal="true"]');
      const ownsActiveModal = activeModal?.classList.contains("file-space-dialog") ?? false;
      if (activeModal && !ownsActiveModal) return;
      if (!ownsActiveModal && document.querySelector(".file-space-ai-popover, .file-space-ai-panel")) return;
      event.preventDefault();
      if (restoreConfirmationEntryId && busyAction === "restore") return;
      if (isEmptyTrashConfirmationOpen && busyAction === "purgeTrash") return;
      if (pendingFileImportConflict && busyAction === "import") return;
      if (isCreateFolderOpen) setCreateFolderOpen(false);
      else if (createFileFormat) {
        if (busyAction === "create") return;
        setCreateFileFormat(null);
      }
      else if (renameFolderId) setRenameFolderId(null);
      else if (deleteFolderId) setDeleteFolderId(null);
      else if (renameFileId) setRenameFileId(null);
      else if (deleteFileId) setDeleteFileId(null);
      else if (restoreConfirmationEntryId) setRestoreConfirmationEntryId(null);
      else if (isEmptyTrashConfirmationOpen) setEmptyTrashConfirmationOpen(false);
      else if (pendingFileImportConflict) setPendingFileImportConflict(null);
      else if (tagFileId) setTagFileId(null);
      else if (isFilterMenuOpen) setFilterMenuOpen(false);
      else if (folderContextMenu) setFolderContextMenu(null);
      else if (fileContextMenu) setFileContextMenu(null);
      else if (contentContextMenu) setContentContextMenu(null);
      else if (isTimelinePanelOpen) closeTimelinePanel();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busyAction, closeTimelinePanel, contentContextMenu, createFileFormat, deleteFileId, deleteFolderId, fileContextMenu, folderContextMenu, isCreateFolderOpen, isEmptyTrashConfirmationOpen, isFilterMenuOpen, isTimelinePanelOpen, pendingFileImportConflict, renameFileId, renameFolderId, restoreConfirmationEntryId, tagFileId]);

  useEffect(() => {
    if (!isFilterMenuOpen) return undefined;
    const closeFilterMenu = (event: PointerEvent) => {
      if (!filterMenuRef.current?.contains(event.target as Node)) setFilterMenuOpen(false);
    };
    window.addEventListener("pointerdown", closeFilterMenu);
    return () => window.removeEventListener("pointerdown", closeFilterMenu);
  }, [isFilterMenuOpen]);

  useEffect(() => {
    if (!folderContextMenu && !fileContextMenu && !contentContextMenu) return undefined;
    const focusFrame = window.requestAnimationFrame(() => {
      contextMenuRef.current?.querySelector<HTMLButtonElement>(
        'button[role="menuitem"]:not(:disabled), button[role="menuitemradio"]:not(:disabled)',
      )?.focus();
    });
    const closeContextMenu = (event: PointerEvent) => {
      if (!contextMenuRef.current?.contains(event.target as Node)) {
        setFolderContextMenu(null);
        setFileContextMenu(null);
        setContentContextMenu(null);
      }
    };
    const closeForLayoutChange = () => {
      setFolderContextMenu(null);
      setFileContextMenu(null);
      setContentContextMenu(null);
    };
    window.addEventListener("pointerdown", closeContextMenu);
    window.addEventListener("resize", closeForLayoutChange);
    window.addEventListener("scroll", closeForLayoutChange, true);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      window.removeEventListener("pointerdown", closeContextMenu);
      window.removeEventListener("resize", closeForLayoutChange);
      window.removeEventListener("scroll", closeForLayoutChange, true);
    };
  }, [contentContextMenu, fileContextMenu, folderContextMenu]);

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
    return new Map(snapshot.folders.map((folder) => [folder.id, folder.fileCount ?? 0]));
  }, [snapshot.folders]);

  const folderContextMenuExpandableIds = useMemo(() => (
    folderContextMenu
      ? expandableFolderIdsInSubtree(snapshot.folders, folderContextMenu.folderId)
      : []
  ), [folderContextMenu, snapshot.folders]);
  const isFolderContextSubtreeFullyExpanded = isFolderSubtreeFullyExpanded(
    expandedFolders,
    folderContextMenuExpandableIds,
  );

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
        fileCount: folder.directFileCount ?? directFiles.length,
        folderCount: folder.childFolderCount ?? (foldersByParent.get(folder.id) ?? []).length,
        previews: directFiles.slice(0, 4),
      });
    });
    return details;
  }, [foldersByParent, snapshot.files, snapshot.folders]);

  const currentFolder = snapshot.folders.find((folder) => folder.id === currentFolderId) ?? null;
  const isTrashView = activeCollection === "trash";
  const folderBeingDragged = internalFolderDrag
    ? snapshot.folders.find((folder) => folder.id === internalFolderDrag.folderId) ?? null
    : null;
  const deletedFile = snapshot.files.find((file) => file.id === deleteFileId) ?? null;
  const trashItemPendingRestore = snapshot.trashItems.find(
    (item) => item.id === restoreConfirmationEntryId,
  ) ?? null;
  const childFolders = foldersByParent.get(currentFolder?.id ?? null) ?? [];
  const allTags = [...snapshot.tags]
    .sort((left, right) => left.localeCompare(right, locale));
  const directFiles = useMemo(() => (
    currentFolder
      ? snapshot.files.filter((file) => file.folderId === currentFolder.id)
      : snapshot.files
  ), [currentFolder, snapshot.files]);
  const normalizedQuery = query.trim();
  const activeSearchResult = searchResult?.key === searchKey ? searchResult.ids : null;
  const updatedAfter = timeFilterStart(timeFilter);
  const matchedFileIds = useMemo(
    () => normalizedQuery && activeSearchResult ? [...activeSearchResult] : [],
    [activeSearchResult, normalizedQuery],
  );
  const createFilePageRequest = useCallback((cursor: FileSpaceFilePageCursor | null) => ({
    folderId: currentFolder?.id ?? null,
    sort: sortOption,
    typeFilter,
    tagFilter: tagFilter === "all" ? null : tagFilter,
    updatedAfter,
    matchIds: matchedFileIds,
    cursor,
    limit: filePageLimit,
  }), [currentFolder?.id, matchedFileIds, sortOption, tagFilter, typeFilter, updatedAfter]);

  useEffect(() => {
    if (!isTauri() || snapshot.rootStatus !== "ready" || isTrashView) return undefined;
    if (normalizedQuery && !activeSearchResult) {
      setFilePageLoading(searchLoading);
      return undefined;
    }
    const sequence = filePageSequenceRef.current + 1;
    filePageSequenceRef.current = sequence;
    const revealRequest = fileRevealNavigationRef.current.pending;
    if (revealRequest) revealRequest.pageReady = false;
    filePageLoadingRef.current = true;
    setFilePageLoading(true);
    setFilePageCursor(null);
    const scroll = contentDropZoneRef.current;
    if (scroll) scroll.scrollTop = 0;
    void invoke<FileSpaceFilePage>("list_file_space_files", {
      request: createFilePageRequest(null),
    }).then((page) => {
      if (sequence !== filePageSequenceRef.current || !lifecycleRef.current.mounted) return;
      const files = fileRevealNavigationRef.current.acceptPage(
        revealRequest, currentFolder?.id ?? null, page.files,
      );
      setSnapshot((current) => ({ ...current, files }));
      setFilePageTotal(page.totalCount);
      setFilePageCursor(page.nextCursor);
      setFileVirtualViewport({ top: 0, bottom: 2_000 });
    }).catch((pageError) => {
      if (sequence === filePageSequenceRef.current && lifecycleRef.current.mounted) {
        if (fileRevealNavigationRef.current.pending === revealRequest) {
          fileRevealNavigationRef.current.cancel();
        }
        setError(`${t("fileSpace.errors.load")} ${errorText(pageError)}`);
      }
    }).finally(() => {
      if (sequence === filePageSequenceRef.current && lifecycleRef.current.mounted) {
        filePageLoadingRef.current = false;
        setFilePageLoading(false);
      }
    });
    return () => {
      if (filePageSequenceRef.current === sequence) filePageSequenceRef.current += 1;
    };
  }, [
    activeCollection,
    activeSearchResult,
    createFilePageRequest,
    filePageRevision,
    isTrashView,
    normalizedQuery,
    searchLoading,
    snapshot.folders,
    snapshot.rootStatus,
    t,
    workspaceGeneration,
  ]);

  const loadMoreFilePage = useCallback(() => {
    if (
      !isTauri()
      || !filePageCursor
      || filePageLoadingRef.current
      || snapshot.files.length >= filePageTotal
    ) return;
    const sequence = filePageSequenceRef.current;
    filePageLoadingRef.current = true;
    setFilePageLoading(true);
    void invoke<FileSpaceFilePage>("list_file_space_files", {
      request: createFilePageRequest(filePageCursor),
    }).then((page) => {
      if (sequence !== filePageSequenceRef.current || !lifecycleRef.current.mounted) return;
      setSnapshot((current) => {
        const existing = new Set(current.files.map((file) => file.id));
        const appended = page.files.filter((file) => !existing.has(file.id));
        return appended.length > 0
          ? { ...current, files: [...current.files, ...appended] }
          : current;
      });
      setFilePageTotal(page.totalCount);
      setFilePageCursor(page.nextCursor);
    }).catch((pageError) => {
      if (sequence === filePageSequenceRef.current && lifecycleRef.current.mounted) {
        setError(`${t("fileSpace.errors.load")} ${errorText(pageError)}`);
      }
    }).finally(() => {
      if (sequence === filePageSequenceRef.current && lifecycleRef.current.mounted) {
        filePageLoadingRef.current = false;
        setFilePageLoading(false);
      }
    });
  }, [createFilePageRequest, filePageCursor, filePageTotal, snapshot.files.length, t]);
  const visibleFolders = childFolders.filter((folder) => (
    !normalizedQuery || (searchScopes.includes("name") && matchesSearch(folder.name, normalizedQuery))
  ));
  const sortedVisibleFiles = useMemo(() => directFiles
    .filter((file) => (
      (!normalizedQuery || activeSearchResult?.has(file.id))
      && (typeFilter === "all" || fileCategory(file) === typeFilter)
      && (tagFilter === "all" || file.tags.includes(tagFilter))
      && (updatedAfter === null || file.updatedAt >= updatedAfter)
    ))
    .sort((left, right) => compareFiles(left, right, sortOption, locale)), [
      activeSearchResult,
      directFiles,
      locale,
      normalizedQuery,
      sortOption,
      tagFilter,
      timeFilter,
      typeFilter,
      updatedAfter,
    ]);
  useEffect(() => {
    if (isTauri() || !import.meta.env.DEV) return;
    const previewParameters = new URLSearchParams(window.location.search);
    if (
      previewParameters.has("fileSpacePreview")
      || previewParameters.has("trashPreview")
      || previewParameters.has("emptyPreview")
    ) {
      setFilePageTotal(sortedVisibleFiles.length);
    }
  }, [sortedVisibleFiles.length]);
  const visibleFileById = useMemo(
    () => new Map(sortedVisibleFiles.map((file) => [file.id, file])),
    [sortedVisibleFiles],
  );
  const visibleFiles = useMemo(() => internalFileDrag
    ? internalFileDrag.orderedIds.flatMap((id) => {
        const file = visibleFileById.get(id);
        return file ? [file] : [];
      })
    : sortedVisibleFiles, [internalFileDrag, sortedVisibleFiles, visibleFileById]);
  const selectedFiles = sortedVisibleFiles.filter((file) => selectedFileIds.has(file.id));
  const selectedFile = selectedFiles.length === 1 ? selectedFiles[0] : null;
  const canReorderVisibleFiles = canActivateFileReorder({
    hasSearch: Boolean(normalizedQuery),
    hasFilters: typeFilter !== "all" || tagFilter !== "all" || timeFilter !== "all",
    usesAutomaticOrder: sortOption === "manual",
    visibleFileCount: sortedVisibleFiles.length,
  });
  const internallyDraggedFile = internalFileDrag
    ? snapshot.files.find((file) => file.id === internalFileDrag.fileId) ?? null
    : null;
  const visibleFileIds = useMemo(
    () => visibleFiles.map((file) => file.id),
    [visibleFiles],
  );
  const fileLayoutItems = useMemo(() => visibleFiles.map((file) => {
    const dimensions = imageDimensionsByFileVersion[imageDimensionsKey(file.id, file.updatedAt)];
    return {
      id: file.id,
      aspectRatio: dimensions?.width && dimensions.height
        ? dimensions.width / dimensions.height
        : defaultFilePreviewAspectRatio(file),
    };
  }), [imageDimensionsByFileVersion, visibleFiles]);

  useLayoutEffect(() => {
    const grid = fileGridRef.current;
    if (!grid) return undefined;

    let animationFrame = 0;
    let disposed = false;
    const measure = () => {
      animationFrame = 0;
      if (disposed) return;
      const containerWidth = grid.getBoundingClientRect().width;
      if (containerWidth <= 0) return;

      const nextLayout = fileLayoutMode === "list"
        ? calculateFileListLayout({
            containerWidth,
            rowHeight: fileListRowHeight,
            verticalGap: fileListVerticalGap,
            previewSize: fileListPreviewSize,
            items: fileLayoutItems,
          })
        : calculateJustifiedFileLayout({
            containerWidth,
            targetPreviewHeight: previewSize,
            horizontalGap: fileLayoutHorizontalGap,
            verticalGap: fileLayoutVerticalGap,
            previewDetailsGap: filePreviewDetailsGap,
            detailsHeight: fileDetailsHeight,
            items: fileLayoutItems,
          });
      measuredFileLayoutRef.current = nextLayout;
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
    measure();

    return () => {
      disposed = true;
      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
    };
  }, [fileDetailsHeight, fileLayoutItems, fileLayoutMode, previewSize]);

  const updateFileVirtualViewport = useCallback(() => {
    const scroll = contentDropZoneRef.current;
    const grid = fileGridRef.current;
    if (!scroll || !grid) return;
    const scrollRect = scroll.getBoundingClientRect();
    const gridRect = grid.getBoundingClientRect();
    const top = Math.max(0, scrollRect.top - gridRect.top - fileVirtualOverscan);
    const bottom = Math.max(top, scrollRect.bottom - gridRect.top + fileVirtualOverscan);
    setFileVirtualViewport((current) => (
      Math.abs(current.top - top) < 1 && Math.abs(current.bottom - bottom) < 1
        ? current
        : { top, bottom }
    ));
    if (scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= fileVirtualOverscan * 1.5) {
      loadMoreFilePage();
    }
  }, [loadMoreFilePage]);

  const scheduleFileVirtualViewportUpdate = useCallback(() => {
    if (fileVirtualViewportFrameRef.current !== null) return;
    fileVirtualViewportFrameRef.current = window.requestAnimationFrame(() => {
      fileVirtualViewportFrameRef.current = null;
      updateFileVirtualViewport();
    });
  }, [updateFileVirtualViewport]);

  useLayoutEffect(() => {
    const scroll = contentDropZoneRef.current;
    const grid = fileGridRef.current;
    if (!scroll || !grid) return undefined;
    scheduleFileVirtualViewportUpdate();
    const resizeObserver = new ResizeObserver(scheduleFileVirtualViewportUpdate);
    resizeObserver.observe(scroll);
    resizeObserver.observe(grid);
    return () => resizeObserver.disconnect();
  }, [fileJustifiedLayout?.height, scheduleFileVirtualViewportUpdate]);

  const renderedFiles = useMemo(() => {
    if (!fileJustifiedLayout) return [];
    return visibleJustifiedFileIds(
      visibleFileIds,
      fileJustifiedLayout,
      fileVirtualViewport.top,
      fileVirtualViewport.bottom,
    ).flatMap((id) => {
      const file = visibleFileById.get(id);
      return file ? [file] : [];
    });
  }, [fileJustifiedLayout, fileVirtualViewport, visibleFileById, visibleFileIds]);

  useLayoutEffect(() => {
    const navigation = fileRevealNavigationRef.current;
    const request = navigation.readyForLayout(currentFolderId);
    const revealFile = request?.file;
    const scroll = contentDropZoneRef.current;
    const grid = fileGridRef.current;
    const placement = revealFile ? fileJustifiedLayout?.placements[revealFile.id] : null;
    if (!request || !revealFile || !scroll || !grid || !placement || !fileJustifiedLayout) return;
    // Layout measurement can enqueue another React commit. Do not complete on
    // placements from the previous page, sort order, or inspector width.
    if (!measuredFileLayoutRef.current
      || !justifiedFileLayoutsEqual(fileJustifiedLayout, measuredFileLayoutRef.current)) return;
    const scrollRect = scroll.getBoundingClientRect();
    const gridRect = grid.getBoundingClientRect();
    const targetTop = gridRect.top - scrollRect.top + scroll.scrollTop + placement.y;
    scroll.scrollTop = Math.max(0, targetTop - Math.max(24, (scroll.clientHeight - fileJustifiedLayout.cardHeight) / 2));
    updateFileVirtualViewport();
    const cardButton = grid.querySelector<HTMLButtonElement>(
      `.file-space-file-card[data-file-id="${CSS.escape(revealFile.id)}"] > button:first-child`,
    );
    navigation.completeWithTarget(
      request,
      cardButton,
      (button) => button.focus({ preventScroll: true }),
      (button) => {
        button.dispatchEvent(new MouseEvent("dblclick", {
          bubbles: true,
          cancelable: true,
          button: 0,
          view: window,
        }));
      },
    );
  }, [currentFolderId, fileJustifiedLayout, fileRevealRevision, renderedFiles, updateFileVirtualViewport]);

  useEffect(() => {
    const visibleIds = selectionVisibilityWithPendingReveal(
      sortedVisibleFiles.map((file) => file.id),
      fileRevealNavigationRef.current.pending?.file.id,
    );
    const nextSelection = pruneSelection(selectedFileIdsRef.current, visibleIds);
    if (!setsEqual(nextSelection, selectedFileIdsRef.current)) {
      updateSelectedFiles(nextSelection);
    }
    if (selectionAnchorRef.current && !nextSelection.has(selectionAnchorRef.current)) {
      selectionAnchorRef.current = null;
    }
    if (nextSelection.size !== 1) {
      setInspectorTimeline(null);
      setInspectorTimelineLoading(false);
    }
  }, [visibleFileIds]);

  useEffect(() => {
    let cancelled = false;
    setInspectorTimeline(null);
    if (!selectedFile || selectedFile.versionCount < 1) {
      setInspectorTimelineLoading(false);
      return () => { cancelled = true; };
    }
    if (!isTauri()) {
      setInspectorTimeline(createVisualTimeline(selectedFile));
      setInspectorTimelineLoading(false);
      return () => { cancelled = true; };
    }
    setInspectorTimelineLoading(true);
    void (async () => {
      try {
        const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: selectedFile.id });
        // Selection only loads inspector data. Replacing the workspace snapshot
        // here also replaces the current paged file window with the root page;
        // that retriggers pagination, resets scrollTop, and makes a click look
        // like a file reorder. Workspace mutations refresh the snapshot through
        // their own explicit paths.
        if (!cancelled && lifecycleRef.current.mounted) setInspectorTimeline(loaded);
      } catch {
        if (!cancelled) setInspectorTimeline(null);
      } finally {
        if (!cancelled) setInspectorTimelineLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [selectedFile?.id, selectedFile?.updatedAt, selectedFile?.versionCount]);

  const openTimeline = async (file: FileSpaceFileRecord, targetVersionId?: string) => {
    if (capabilities?.history === false) return;
    // External catalogues can load history on demand without an upfront count.
    if (file.versionCount < 1 && !externalWorkspace?.source && inspectorTimeline?.fileId !== file.id) return;
    if (!isTimelinePanelOpen && !timelineBadgeReturnFocusRef.current) {
      timelineBadgeReturnFocusRef.current = document.activeElement instanceof HTMLElement
        ? document.activeElement : null;
    }
    const requestId = timelineRequestRef.current + 1;
    timelineRequestRef.current = requestId;
    versionPreviewRequestRef.current += 1;
    setFileContextMenu(null);
    setTimelineBusy(true);
    setOpeningTimelineFileId(file.id);
    setError(null);
    try {
      const loaded = !isTauri()
        ? createVisualTimeline(file)
        : await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId: file.id });
      if (!loaded || requestId !== timelineRequestRef.current || !lifecycleRef.current.mounted) return;
      // Opening history is read-only: keep the current paged list and its scroll position.
      if (selectedFileIdsRef.current.size === 1 && selectedFileIdsRef.current.has(file.id)) {
        setInspectorTimeline(loaded);
      }
      setTimelineFile(file);
      setTimeline(loaded);
      const selectedId = loaded.versions.some((version) => version.id === targetVersionId)
        ? targetVersionId! : loaded.currentVersionId;
      setSelectedVersionId(selectedId);
      setVersionPreview(null);
      const comparison = defaultVersionComparison(loaded.versions, selectedId);
      setDiffBeforeVersionId(comparison?.beforeVersionId ?? null);
      setDiffAfterVersionId(comparison?.afterVersionId ?? null);
      setVersionDiffStatus(comparison ? "loading" : "idle");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      setTimelinePanelOpen(true);
      return true;
    } catch (timelineError) {
      if (requestId === timelineRequestRef.current && lifecycleRef.current.mounted) {
        setError(errorText(timelineError));
      }
    } finally {
      if (requestId === timelineRequestRef.current && lifecycleRef.current.mounted) {
        setTimelineBusy(false);
        setOpeningTimelineFileId(null);
      }
    }
  };

  const selectTimelineVersion = async (version: TaskFileVersionRecord) => {
    if (!timeline) return;
    const fileId = timeline.fileId;
    const timelineRequestId = timelineRequestRef.current;
    const previewRequestId = versionPreviewRequestRef.current + 1;
    versionPreviewRequestRef.current = previewRequestId;
    setSelectedVersionId(version.id);
    const currentVersion = timeline.versions.find((candidate) => candidate.id === timeline.currentVersionId);
    if (currentVersion && version.id !== currentVersion.id) {
      if (version.versionNumber < currentVersion.versionNumber) {
        setDiffBeforeVersionId(version.id);
        setDiffAfterVersionId(currentVersion.id);
      } else {
        setDiffBeforeVersionId(currentVersion.id);
        setDiffAfterVersionId(version.id);
      }
    } else {
      const comparison = defaultVersionComparison(timeline.versions, timeline.currentVersionId);
      setDiffBeforeVersionId(comparison?.beforeVersionId ?? null);
      setDiffAfterVersionId(comparison?.afterVersionId ?? null);
    }
    if (!isTauri()) {
      setVersionPreview(import.meta.env.DEV && timelineFile ? createVisualVersionContent(timelineFile, version) : null);
      setTimelineBusy(false);
      return;
    }
    if (!supportsTextPreview(version)) {
      setVersionPreview(null);
      setTimelineBusy(false);
      return;
    }
    setTimelineBusy(true);
    try {
      const bytes = await invoke<number[]>("read_task_file_version", {
        fileId,
        versionId: version.id,
      });
      if (
        previewRequestId === versionPreviewRequestRef.current
        && timelineRequestId === timelineRequestRef.current
        && lifecycleRef.current.mounted
      ) {
        setVersionPreview(new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes)));
      }
    } catch {
      if (
        previewRequestId === versionPreviewRequestRef.current
        && timelineRequestId === timelineRequestRef.current
        && lifecycleRef.current.mounted
      ) {
        setVersionPreview(null);
      }
    } finally {
      if (
        previewRequestId === versionPreviewRequestRef.current
        && timelineRequestId === timelineRequestRef.current
        && lifecycleRef.current.mounted
      ) {
        setTimelineBusy(false);
      }
    }
  };

  const dismissVersionNotification = () => {
    setVersionNotifications((current) => current.slice(1));
  };

  const viewVersionNotification = async (notification = versionNotifications[0]) => {
    if (!notification) return;
    let file = snapshot.files.find((candidate) => candidate.id === notification.fileId);
    if (!file && isTauri()) {
      try {
        const page = await invoke<FileSpaceFilePage>("list_file_space_files", {
          request: { folderId: null, sort: "updatedDesc", typeFilter: "all", tagFilter: null,
            updatedAfter: null, matchIds: [notification.fileId], cursor: null, limit: 1 },
        });
        [file] = page.files;
      } catch {
        file = undefined;
      }
    }
    if (!file) {
      setError(t("fileSpace.versionNotification.unavailable"));
      return;
    }
    revealFileInWorkspace(file);
    if (await openTimeline(file, notification.versionId)) {
      setVersionNotifications((current) => current.filter((item) => item.versionId !== notification.versionId));
    }
  };

  useEffect(() => {
    if (!importConflictFeedback && versionNotification && claimFirstVersionChange(versionNotification)) {
      setFirstVersionNotificationId(versionNotification.versionId);
    }
  }, [importConflictFeedback, versionNotification]);

  useEffect(() => {
    const viewSavedChange = (event: Event) => {
      const notification = (event as CustomEvent<FileSpaceVersionNotification>).detail;
      if (notification?.fileId && notification.versionId) void viewVersionNotification(notification);
    };
    window.addEventListener(viewVersionChangeEvent, viewSavedChange);
    return () => window.removeEventListener(viewVersionChangeEvent, viewSavedChange);
  });

  const viewIdenticalImportFile = async () => {
    const notice = importConflictFeedback;
    const target = notice?.files[0];
    if (!target) return;

    let latestSnapshot = snapshot;
    let file = latestSnapshot.files.find((candidate) => candidate.id === target.fileId);
    if (!file && isTauri()) {
      try {
        latestSnapshot = await invoke<FileSpaceSnapshot>("get_file_space_snapshot");
        setSnapshot(latestSnapshot);
        file = latestSnapshot.files.find((candidate) => candidate.id === target.fileId);
      } catch {
        file = undefined;
      }
    }
    if (!file) {
      setImportConflictFeedback(null);
      setError(t("fileSpace.importConflict.unavailable"));
      return;
    }

    setImportConflictFeedback(null);
    setActiveCollection("files");
    setCurrentFolderId(file.folderId);
    setQuery("");
    setTypeFilter("all");
    setTagFilter("all");
    setTimeFilter("all");
    setFilterMenuOpen(false);
    setInspectorVisible(true);
    updateSelectedFiles(new Set([file.id]), file.id);

    const fileId = file.id;
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        const button = document.querySelector<HTMLButtonElement>(
          `.file-space-file-card[data-file-id="${CSS.escape(fileId)}"] > button`,
        );
        button?.scrollIntoView({
          behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
          block: "center",
          inline: "center",
        });
        button?.focus({ preventScroll: true });
      });
    });
  };

  const setCurrentTimelineVersion = async (versionId: string) => {
    if (!timeline || !isTauri()) return;
    const fileId = timeline.fileId;
    const requestId = timelineRequestRef.current + 1;
    const mutationRequestId = timelineMutationRequestRef.current + 1;
    timelineRequestRef.current = requestId;
    timelineMutationRequestRef.current = mutationRequestId;
    versionPreviewRequestRef.current += 1;
    setTimelineBusy(true);
    setVersionPreview(null);
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("set_current_task_file_version", {
        fileId,
        versionId,
      });
      if (
        mutationRequestId !== timelineMutationRequestRef.current
        || !lifecycleRef.current.mounted
      ) return;
      setSnapshot(updated);
      if (requestId !== timelineRequestRef.current || !lifecycleRef.current.mounted) return;
      const loaded = await invoke<TaskFileTimelineRecord>("get_task_file_timeline", { fileId });
      if (
        mutationRequestId !== timelineMutationRequestRef.current
        || requestId !== timelineRequestRef.current
        || !lifecycleRef.current.mounted
      ) return;
      setTimeline(loaded);
      setInspectorTimeline(loaded);
      setSelectedVersionId(loaded.currentVersionId);
      const comparison = defaultVersionComparison(loaded.versions, loaded.currentVersionId);
      setDiffBeforeVersionId(comparison?.beforeVersionId ?? null);
      setDiffAfterVersionId(comparison?.afterVersionId ?? null);
      setVersionDiffStatus(comparison ? "loading" : "idle");
      setVersionDiffResult(null);
      setVersionDiffError(null);
      setTimelineFile(updated.files.find((file) => file.id === fileId) ?? timelineFile);
    } catch (versionError) {
      if (
        mutationRequestId === timelineMutationRequestRef.current
        && lifecycleRef.current.mounted
      ) {
        setError(errorText(versionError));
      }
    } finally {
      if (requestId === timelineRequestRef.current && lifecycleRef.current.mounted) {
        setTimelineBusy(false);
      }
    }
  };

  const swapComparedVersions = () => {
    if (!diffBeforeVersionId || !diffAfterVersionId) return;
    setDiffBeforeVersionId(diffAfterVersionId);
    setDiffAfterVersionId(diffBeforeVersionId);
  };

  const folderIsActuallyEmpty = childFolders.length === 0 && filePageTotal === 0;
  const showEmptyDropZone = folderIsActuallyEmpty
    && !normalizedQuery
    && typeFilter === "all"
    && tagFilter === "all"
    && timeFilter === "all";
  const workspaceTitle = isTrashView
    ? t("fileSpace.trash.title")
    : currentFolder?.name ?? snapshot.rootName ?? t("fileSpace.title");

  const applyWorkspaceMutation = useCallback((mutation: FileSpaceWorkspaceMutation<FileSpaceSnapshot>) => {
    // The extension remounts the workbench for its next selection; do not write
    // a local snapshot into the outgoing source or erase its remembered folder.
    if (externalWorkspace) { setWorkspaceDirectory(mutation.directory); return; }
    aiSourceNavigationSequenceRef.current += 1;
    fileRevealNavigationRef.current.cancel();
    setWorkspaceDirectory(mutation.directory);
    setSnapshot(mutation.snapshot);
    setFilePageTotal(mutation.snapshot.fileCount);
    setFilePageCursor(null);
    setExpandedFolders(new Set());
    setCurrentFolderId(null);
    window.localStorage.removeItem(currentFolderStorageKey);
    setActiveCollection("files");
    setQuery("");
    setSearchResult(null);
    setSearchLoading(false);
    setTypeFilter("all");
    setTagFilter("all");
    setTimeFilter("all");
    setFilterMenuOpen(false);
    setGlobalSearchOpen(false);
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setTimelinePanelOpen(false);
    setTimeline(null);
    setTimelineFile(null);
    setInspectorTimeline(null);
    setInspectorTimelineLoading(false);
    updateSelectedFiles(new Set(), null);
    setVersionNotifications([]);
    setImportConflictFeedback(null);
    setTrashFeedback(null);
    setError(null);
    setWorkspaceGeneration((generation) => generation + 1);
    setFilePageRevision((revision) => revision + 1);
  }, []);
  const hasActiveFilters = typeFilter !== "all"
    || tagFilter !== "all"
    || timeFilter !== "all"
    || sortOption !== "manual"
    || searchScopes.length !== 3;
  const hasRestrictiveFilters = typeFilter !== "all"
    || tagFilter !== "all"
    || timeFilter !== "all";
  const chooseStorageRoot = async (mode: RootSetupMode, returnFocusTarget?: HTMLElement) => {
    if (mode === "import" && returnFocusTarget) {
      importExistingTriggerRef.current = returnFocusTarget;
    }
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
      if (mode === "import") {
        setPendingImportPath(Array.isArray(selected) ? selected[0] ?? null : selected);
        return;
      }
      const configured = await invoke<FileSpaceSnapshot>("configure_file_space_root", {
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

  const confirmExistingFolderImport = async () => {
    if (!pendingImportPath) return;
    const operation = beginOperation("configure");
    if (!operation) return;
    const requestId = createImportRequestId();
    importRequestRef.current = requestId;
    setRootSetupMode("import");
    setSetupCancelled(false);
    setImportFeedback({
      requestId,
      phase: "scanning",
      processed: 0,
      total: 0,
      currentName: null,
    });
    setError(null);
    try {
      await waitForCommittedPaint();
      if (!canCommitOperation(operation)) return;
      const configured = await invoke<FileSpaceSnapshot>("import_existing_file_space_root", {
        requestId,
        path: pendingImportPath,
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(configured);
      setCurrentFolderId(null);
      setExpandedFolders(new Set(configured.folders.map((folder) => folder.id)));
      setPendingImportPath(null);
      setImportFeedback(null);
    } catch (configureError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.initializeImport")} ${errorText(configureError)}`);
      }
    } finally {
      if (importRequestRef.current === requestId) importRequestRef.current = null;
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

  const createTextFile = async () => {
    if (!createFileFormat || !createFileName.trim()) return;
    if (!isTauri()) {
      setError(`${t("fileSpace.errors.createFile")} Tauri desktop runtime is required.`);
      return;
    }
    const operation = beginOperation("create");
    if (!operation) return;
    setError(null);
    try {
      const result = await invoke<FileSpaceCreatedFileResult>("create_file_space_text_file", {
        parentId: createFileParentId,
        name: `${createFileName.trim()}.${createFileFormat}`,
        format: createFileFormat,
      });
      if (!canCommitOperation(operation)) return;
      fileRevealNavigationRef.current.start(result.file);
      setSnapshot(result.snapshot);
      setFilePageTotal(result.snapshot.fileCount);
      setFilePageCursor(null);
      updateSelectedFiles(new Set([result.file.id]), result.file.id);
      setCreateFileFormat(null);
      setCreateFileParentId(null);
      setCreateFileName("");
      setFilePageRevision((revision) => revision + 1);
    } catch (createError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.createFile")} ${errorText(createError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const openCreateTextFile = (parentId: string | null) => {
    if (!canWrite) return;
    if (operationRef.current) return;
    rememberDialogReturnFocus(contentDropZoneRef.current);
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setContentContextMenu(null);
    setCreateFileParentId(parentId);
    setCreateFileFormat("md");
    setCreateFileName(t("fileSpace.createFile.defaultName"));
  };

  const selectCreateTextFileFormat = (format: NewTextFileFormat) => {
    setCreateFileFormat(format);
  };

  const openCreateFolder = (parentId: string | null) => {
    if (!canWrite) return;
    if (operationRef.current) return;
    rememberDialogReturnFocus();
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setContentContextMenu(null);
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
      const inspection = await invoke<FileSpaceDroppedFileConflictInspection>(
        "inspect_file_space_import_conflicts",
        {
          folderId: destinationFolderId,
          paths: selectedPaths,
        },
      );
      if (!canCommitOperation(operation)) return;
      const conflicts = inspection.conflicts.filter((conflict) => !conflict.identical);
      const identicalFiles = inspection.conflicts
        .filter((conflict) => conflict.identical)
        .map((conflict) => ({
          fileId: conflict.existingFileId,
          fileName: conflict.fileName,
        }));
      if (conflicts.length > 0) {
        rememberDialogReturnFocus(contentDropZoneRef.current);
        setImportFeedback(null);
        setPendingFileImportConflict({
          requestId,
          folderId: destinationFolderId,
          source: { kind: "paths", paths: selectedPaths },
          conflicts,
          identicalFiles,
        });
        return;
      }
      const imported = await invoke<FileSpaceSnapshot>("import_file_space_files", {
        requestId,
        folderId: destinationFolderId,
        paths: selectedPaths,
        conflictAction: "rename",
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(imported);
      setExpandedFolders(new Set(imported.folders.map((folder) => folder.id)));
      if (identicalFiles.length > 0) {
        setImportConflictFeedback({ files: identicalFiles });
        setImportFeedback(null);
      } else {
        setImportFeedback((current) => current?.requestId === requestId
          ? { ...current, phase: "completed", processed: current.total || current.processed }
          : current);
      }
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
      const inspection = await invoke<FileSpaceDroppedFileConflictInspection>(
        "inspect_file_space_dropped_conflicts",
        {
          folderId: destinationFolderId,
          files,
        },
      );
      if (!canCommitOperation(operation)) return;
      const conflicts = inspection.conflicts.filter((conflict) => !conflict.identical);
      const identicalFiles = inspection.conflicts
        .filter((conflict) => conflict.identical)
        .map((conflict) => ({
          fileId: conflict.existingFileId,
          fileName: conflict.fileName,
        }));
      if (conflicts.length > 0) {
        rememberDialogReturnFocus(contentDropZoneRef.current);
        setImportFeedback(null);
        setPendingFileImportConflict({
          requestId,
          folderId: destinationFolderId,
          source: { kind: "droppedFiles", files },
          conflicts,
          identicalFiles,
        });
        return;
      }
      const imported = await invoke<FileSpaceSnapshot>("import_file_space_dropped_files", {
        requestId,
        folderId: destinationFolderId,
        files,
        conflictAction: "rename",
      });
      if (!canCommitOperation(operation)) return;
      setSnapshot(imported);
      setExpandedFolders(new Set(imported.folders.map((folder) => folder.id)));
      if (identicalFiles.length > 0) {
        setImportConflictFeedback({ files: identicalFiles });
        setImportFeedback(null);
      } else {
        setImportFeedback((current) => current?.requestId === requestId
          ? { ...current, phase: "completed", processed: current.total || current.processed, currentName: null }
          : current);
      }
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

  const resolveFileImportConflict = async (action: FileImportConflictAction) => {
    const pending = pendingFileImportConflict;
    if (!pending || !isTauri()) return;
    const operation = beginOperation("import");
    if (!operation) return;
    importRequestRef.current = pending.requestId;
    setError(null);
    setImportFeedback({
      requestId: pending.requestId,
      phase: "importing",
      processed: 0,
      total: pending.source.kind === "paths"
        ? pending.source.paths.length
        : pending.source.files.length,
      currentName: null,
    });
    try {
      const imported = pending.source.kind === "paths"
        ? await invoke<FileSpaceSnapshot>("import_file_space_files", {
          requestId: pending.requestId,
          folderId: pending.folderId,
          paths: pending.source.paths,
          conflictAction: action,
        })
        : await invoke<FileSpaceSnapshot>("import_file_space_dropped_files", {
          requestId: pending.requestId,
          folderId: pending.folderId,
          files: pending.source.files,
          conflictAction: action,
        });
      if (!canCommitOperation(operation)) return;
      setSnapshot(imported);
      setExpandedFolders(new Set(imported.folders.map((folder) => folder.id)));
      if (pending.identicalFiles.length > 0) {
        setImportConflictFeedback({ files: pending.identicalFiles });
        setImportFeedback(null);
      } else {
        setImportFeedback((current) => current?.requestId === pending.requestId
          ? {
            ...current,
            phase: "completed",
            processed: current.total || current.processed,
            currentName: null,
          }
          : current);
      }
      setPendingFileImportConflict(null);
    } catch (importError) {
      if (canCommitOperation(operation)) {
        const message = errorText(importError);
        if (message.includes("IMPORT_CANCELLED:")) {
          setImportFeedback((current) => current?.requestId === pending.requestId
            ? { ...current, phase: "cancelled", error: undefined, currentName: null }
            : current);
          setPendingFileImportConflict(null);
        } else {
          setImportFeedback((current) => current?.requestId === pending.requestId
            ? { ...current, phase: "failed", error: message, currentName: null }
            : current);
          setError(`${t("fileSpace.errors.import")} ${message}`);
        }
      }
    } finally {
      if (importRequestRef.current === pending.requestId) importRequestRef.current = null;
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

  const nativeDropBlocked = !canWrite || Boolean(
    applicationExtension?.active ||
    internalFileDrag ||
    internalFolderDrag ||
    selectionMarquee ||
    busyAction ||
    operationRef.current ||
    folderContextMenu ||
    fileContextMenu ||
    contentContextMenu ||
    isFilterMenuOpen ||
    isCreateFolderOpen ||
    createFileFormat ||
    renameFolderId ||
    deleteFolderId ||
    renameFileId ||
    deleteFileId ||
    restoreConfirmationEntryId ||
    isEmptyTrashConfirmationOpen ||
    pendingFileImportConflict ||
    tagFileId ||
    createDialogPresence.mounted ||
    createFileDialogPresence.mounted ||
    renameDialogPresence.mounted ||
    deleteDialogPresence.mounted ||
    renameFileDialogPresence.mounted ||
    deleteFileDialogPresence.mounted ||
    restoreTrashDialogPresence.mounted ||
    emptyTrashDialogPresence.mounted ||
    importConflictDialogPresence.mounted ||
    tagDialogPresence.mounted
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
    contextMenuReturnFocusRef.current = event.currentTarget instanceof HTMLElement
      ? event.currentTarget
      : document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    setContentContextMenu(null);
    const menuWidth = 190;
    const hasChildFolders = (foldersByParent.get(folderId) ?? []).length > 0;
    const menuHeight = hasChildFolders ? 162 : 128;
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
    contextMenuReturnFocusRef.current = event.currentTarget instanceof HTMLElement
      ? event.currentTarget
      : document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    nativeDropBlockedRef.current = true;
    const menuWidth = 218;
    const file = snapshot.files.find((item) => item.id === fileId);
    const menuHeight = file?.versionCount || externalWorkspace?.source ? 280 : 236;
    updateSelectedFiles(new Set([fileId]), fileId);
    setFolderContextMenu(null);
    setContentContextMenu(null);
    setFileContextMenu({
      fileId,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
    });
  };

  const openContentContextMenu = (event: React.MouseEvent<HTMLDivElement>) => {
    if (operationRef.current) return;
    const target = event.target;
    if (
      target instanceof Element
      && target.closest("button, input, textarea, [role='menu'], [aria-modal='true']")
    ) return;
    event.preventDefault();
    event.stopPropagation();
    contextMenuReturnFocusRef.current = event.currentTarget;
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setFileContextMenu(null);
    const menuWidth = 228;
    const menuHeight = 224;
    setContentContextMenu({
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
    });
  };

  const closeContextMenusAndRestoreFocus = () => {
    setFolderContextMenu(null);
    setFileContextMenu(null);
    setContentContextMenu(null);
    const target = contextMenuReturnFocusRef.current;
    contextMenuReturnFocusRef.current = null;
    if (target?.isConnected) window.requestAnimationFrame(() => target.focus({ preventScroll: true }));
  };

  const handleContextMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(
      'button[role="menuitem"]:not(:disabled), button[role="menuitemradio"]:not(:disabled)',
    ));
    if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      closeContextMenusAndRestoreFocus();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) || items.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const currentIndex = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Home") items[0].focus();
    else if (event.key === "End") items.at(-1)?.focus();
    else if (event.key === "ArrowDown") items[(currentIndex + 1 + items.length) % items.length].focus();
    else items[(currentIndex <= 0 ? items.length : currentIndex) - 1].focus();
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
    updateSortOption("manual");
    setSearchScopes(["name", "content", "tag"]);
  };

  const openTagDialog = (fileId: string) => {
    if (!canWrite) return;
    if (operationRef.current) return;
    const file = snapshot.files.find((item) => item.id === fileId);
    if (!file) return;
    rememberDialogReturnFocus(document.querySelector<HTMLElement>(`[data-file-id="${CSS.escape(fileId)}"] > button:first-child`));
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

  const selectFileFromPointer = (
    event: React.PointerEvent<HTMLButtonElement>,
    fileId: string,
  ) => {
    const currentIds = selectedFileIdsRef.current;
    const orderedIds = sortedVisibleFiles.map((file) => file.id);
    const toggle = event.metaKey || event.ctrlKey;
    const range = event.shiftKey;
    const collapseOnClick = !toggle && !range && currentIds.has(fileId) && currentIds.size > 1;
    let nextIds = currentIds;

    if (!collapseOnClick) {
      const next = resolveFileClickSelection({
        currentIds,
        orderedIds,
        targetId: fileId,
        anchorId: selectionAnchorRef.current,
        toggle,
        range,
      });
      nextIds = next.ids;
      updateSelectedFiles(next.ids, next.anchorId);
    }

    pointerSelectionIntentRef.current = { fileId, collapseOnClick };
    const dragIds = nextIds.has(fileId)
      ? orderedSelection(orderedIds, nextIds)
      : [fileId];
    return dragIds.length > 0 ? dragIds : [fileId];
  };

  const handleFileClick = (event: React.MouseEvent<HTMLButtonElement>, fileId: string) => {
    const intent = pointerSelectionIntentRef.current;
    pointerSelectionIntentRef.current = null;
    if (suppressNextFileClickRef.current) {
      suppressNextFileClickRef.current = false;
      return;
    }
    if (event.detail === 0) {
      const next = resolveFileClickSelection({
        currentIds: selectedFileIdsRef.current,
        orderedIds: sortedVisibleFiles.map((file) => file.id),
        targetId: fileId,
        anchorId: selectionAnchorRef.current,
        toggle: event.metaKey || event.ctrlKey,
        range: event.shiftKey,
      });
      updateSelectedFiles(next.ids, next.anchorId);
      return;
    }
    if (intent?.fileId === fileId && intent.collapseOnClick) {
      updateSelectedFiles(new Set([fileId]), fileId);
    }
  };

  const startFileDragOut = async (
    target: HTMLButtonElement,
    fileId: string,
    immediate = false,
    preparedPreview: PreparedFileDragPreview | null = null,
  ) => {
    if (!isTauri() || operationRef.current || fileDragStartingRef.current) return;
    if (fileDragReleaseTimerRef.current !== null) {
      window.clearTimeout(fileDragReleaseTimerRef.current);
      fileDragReleaseTimerRef.current = null;
    }
    fileDragStartingRef.current = true;
    setFileContextMenu(null);
    setError(null);
    // Command+Tab must start the system session before the app loses focus, so
    // this path never waits for an asynchronous DOM snapshot.
    const previewBytes = immediate
      ? preparedPreview?.immediateBytes
        ?? imageDragPreviewBytes(target)
        ?? fallbackFileDragPreviewBytes(target)
      : await (preparedPreview?.promise ?? fileDragPreviewBytes(target));
    if (!lifecycleRef.current.mounted || !fileDragStartingRef.current) return;
    void invoke("start_file_space_drag_out", {
      fileId,
      previewBytes,
      skipGeneratedPreview: immediate && previewBytes !== null,
    })
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

  const clearDeferredFileDragCancellation = () => {
    if (fileDragCancelTimerRef.current === null) return;
    window.clearTimeout(fileDragCancelTimerRef.current);
    fileDragCancelTimerRef.current = null;
  };

  const promoteToNativeFileDrag = (gesture: FileDragGesture, immediate = false) => {
    clearDeferredFileDragCancellation();
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
    suppressNextFileClickRef.current = true;
    window.setTimeout(() => { suppressNextFileClickRef.current = false; }, 0);
    if (gesture.fileIds.length !== 1) {
      return;
    }
    if (!isTauri()) return;
    void startFileDragOut(gesture.target, gesture.fileId, immediate, gesture.preview);
  };

  const resolveFileFolderDropTarget = (
    clientX: number,
    clientY: number,
    fileIds: string[],
  ): FileFolderDropTarget | null => {
    const element = document.elementFromPoint(clientX, clientY);
    if (!(element instanceof Element)) return null;
    const sourceFiles = fileIds.flatMap((fileId) => {
      const file = snapshot.files.find((item) => item.id === fileId);
      return file ? [file] : [];
    });
    const folderTarget = element.closest<HTMLElement>("[data-file-folder-drop], [data-folder-tree-id]");
    const targetId = folderTarget?.dataset.fileFolderDrop ?? folderTarget?.dataset.folderTreeId;
    if (targetId && snapshot.folders.some((folder) => folder.id === targetId)) {
      return {
        folderId: targetId,
        targetId,
        kind: "folder",
        isCurrent: sourceFiles.length > 0 && sourceFiles.every((file) => file.folderId === targetId),
      };
    }
    if (element.closest<HTMLElement>("[data-file-root-drop]")) {
      return {
        folderId: null,
        targetId: null,
        kind: "root",
        isCurrent: sourceFiles.length > 0 && sourceFiles.every((file) => file.folderId === null),
      };
    }
    return null;
  };

  const beginFileDragGesture = (
    event: React.PointerEvent<HTMLButtonElement>,
    fileId: string,
  ) => {
    if (event.button !== 0 || operationRef.current) return;
    const fileIds = selectFileFromPointer(event, fileId);
    if (!canWrite) return;
    const card = event.currentTarget.closest<HTMLElement>(".file-space-file-card");
    const rect = card?.getBoundingClientRect() ?? event.currentTarget.getBoundingClientRect();
    fileDragGestureRef.current = {
      fileId,
      fileIds,
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
      // Generating the high-fidelity native drag image walks and rasterizes the
      // card DOM. Keep clicks and double-clicks on the zero-work path; prepare
      // it only after pointer movement proves this is an actual drag.
      preview: null,
    };
  };

  const continueFileDragGesture = (event: PointerEvent) => {
    const gesture = fileDragGestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    const fileId = gesture.fileId;
    if ((event.buttons & 1) === 0) {
      cancelFileDragGesture(event.pointerId);
      return;
    }
    const dragDistance = Math.hypot(
      event.clientX - gesture.startX,
      event.clientY - gesture.startY,
    );
    if (dragDistance < fileDragThreshold) {
      return;
    }
    const hasReorderMovement = hasMeaningfulFileReorderMovement({
      startX: gesture.startX,
      startY: gesture.startY,
      currentX: event.clientX,
      currentY: event.clientY,
      minimumDistance: fileReorderCommitThreshold,
    });
    if (hasReorderMovement && isTauri() && gesture.fileIds.length === 1 && !gesture.preview) {
      gesture.preview = prepareFileDragPreview(gesture.target);
    }
    event.preventDefault();
    event.stopPropagation();
    if (
      event.metaKey
      && decideFileDragHandoff({
        phase: "reordering",
        fileCount: gesture.fileIds.length,
        desktop: isTauri(),
        trigger: "command",
      }) === "promote"
    ) {
      promoteToNativeFileDrag(gesture, true);
      return;
    }
    if (pointerReachedWindowEdge(event.clientX, event.clientY)) {
      promoteToNativeFileDrag(gesture);
      return;
    }

    const current = internalFileDragRef.current;
    const folderTarget = resolveFileFolderDropTarget(event.clientX, event.clientY, gesture.fileIds);
    if (gesture.phase === "pending" || !current) {
      gesture.phase = "reordering";
      suppressNextFileClickRef.current = true;
      const started: InternalFileDrag = {
        fileId,
        fileIds: gesture.fileIds,
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
      : grid
        && pointerInGrid
        && canReorderVisibleFiles
        && hasReorderMovement
        && current.fileIds.length === 1
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

  const moveFiles = async (fileIds: string[], folderId: string | null) => {
    const uniqueIds = [...new Set(fileIds)];
    const sourceFiles = uniqueIds.flatMap((fileId) => {
      const file = snapshot.files.find((item) => item.id === fileId);
      return file ? [file] : [];
    });
    if (sourceFiles.length === 0) return;
    const destinationName = folderId
      ? snapshot.folders.find((folder) => folder.id === folderId)?.name
      : t("fileSpace.moveFeedback.root");
    if (!destinationName) return;
    setFileMoveFeedback(null);
    if (sourceFiles.every((file) => file.folderId === folderId)) {
      setFileMoveFeedback({
        fileName: sourceFiles.length === 1 ? sourceFiles[0].name : null,
        fileCount: sourceFiles.length,
        destinationName,
        status: "unchanged",
      });
      return;
    }
    const operation = beginOperation("moveFile");
    if (!operation) return;
    setError(null);
    try {
      let result: FileMoveResult;
      if (!isTauri()) {
        const destinationPath = folderId
          ? snapshot.folders.find((folder) => folder.id === folderId)?.relativePath ?? ""
          : "";
        const movedIds = sourceFiles.filter((file) => file.folderId !== folderId).map((file) => file.id);
        const unchangedIds = sourceFiles.filter((file) => file.folderId === folderId).map((file) => file.id);
        const movedIdSet = new Set(movedIds);
        result = {
          snapshot: {
            ...snapshot,
            files: snapshot.files.map((file) => movedIdSet.has(file.id) ? {
              ...file,
              folderId,
              relativePath: destinationPath ? `${destinationPath}/${file.name}` : file.name,
              updatedAt: Date.now(),
            } : file),
          },
          movedIds,
          unchangedIds,
          failed: [],
        };
      } else {
        result = await invoke<FileMoveResult>("move_file_space_files", {
          fileIds: uniqueIds,
          folderId,
        });
      }
      if (!canCommitOperation(operation)) return;
      setSnapshot(result.snapshot);
      setImportFeedback(null);
      if (result.movedIds.length > 0) {
        const movedFile = sourceFiles.find((file) => file.id === result.movedIds[0]);
        setFileMoveFeedback({
          fileName: result.movedIds.length === 1 ? movedFile?.name ?? null : null,
          fileCount: result.movedIds.length,
          destinationName,
          status: "moved",
        });
      } else if (result.unchangedIds.length > 0 && result.failed.length === 0) {
        setFileMoveFeedback({
          fileName: result.unchangedIds.length === 1 ? sourceFiles[0]?.name ?? null : null,
          fileCount: result.unchangedIds.length,
          destinationName,
          status: "unchanged",
        });
      }
      if (result.failed.length > 0) {
        setError(`${t("fileSpace.errors.partialMove", {
          moved: result.movedIds.length,
          total: sourceFiles.length,
          failed: result.failed.length,
        })} ${result.failed[0].message}`);
      }
      if (folderId && result.movedIds.length > 0) {
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
    const folderTarget = resolveFileFolderDropTarget(clientX, clientY, gesture.fileIds);
    const gridRect = fileGridRef.current?.getBoundingClientRect();
    const droppedInGrid = Boolean(
      gridRect
      && clientX >= gridRect.left
      && clientX <= gridRect.right
      && clientY >= gridRect.top
      && clientY <= gridRect.bottom,
    );
    clearDeferredFileDragCancellation();
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
    if (drag) {
      suppressNextFileClickRef.current = true;
      window.setTimeout(() => { suppressNextFileClickRef.current = false; }, 0);
    }
    if (drag && folderTarget) {
      void moveFiles(drag.fileIds, folderTarget.folderId);
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
    clearDeferredFileDragCancellation();
    fileDragGestureRef.current = null;
    cancelInternalFileDrag();
    suppressNextFileClickRef.current = false;
    pointerSelectionIntentRef.current = null;
  };

  useEffect(() => {
    const trackGesture = (event: PointerEvent) => continueFileDragGesture(event);
    const finishGesture = (event: PointerEvent) => (
      finishFileDragGesture(event.pointerId, event.clientX, event.clientY)
    );
    const deferCancelledGesture = (event: PointerEvent) => {
      const gesture = fileDragGestureRef.current;
      if (!gesture || gesture.pointerId !== event.pointerId) return;
      clearDeferredFileDragCancellation();
      fileDragCancelTimerRef.current = window.setTimeout(() => {
        fileDragCancelTimerRef.current = null;
        const current = fileDragGestureRef.current;
        if (!current || current.pointerId !== event.pointerId) return;
        if (
          !document.hasFocus()
          && decideFileDragHandoff({
            phase: current.phase,
            fileCount: current.fileIds.length,
            desktop: isTauri(),
            trigger: "blur",
          }) === "promote"
        ) {
          promoteToNativeFileDrag(current, true);
          return;
        }
        cancelFileDragGesture(event.pointerId);
      }, 0);
    };
    const handoffOrCancelOnBlur = () => {
      const gesture = fileDragGestureRef.current;
      if (!gesture) return;
      const decision = decideFileDragHandoff({
        phase: gesture.phase,
        fileCount: gesture.fileIds.length,
        desktop: isTauri(),
        trigger: "blur",
      });
      if (decision === "promote") {
        promoteToNativeFileDrag(gesture, true);
        return;
      }
      cancelFileDragGesture();
    };
    window.addEventListener("pointermove", trackGesture, true);
    window.addEventListener("pointerup", finishGesture, true);
    window.addEventListener("pointercancel", deferCancelledGesture, true);
    window.addEventListener("blur", handoffOrCancelOnBlur);
    return () => {
      window.removeEventListener("pointermove", trackGesture, true);
      window.removeEventListener("pointerup", finishGesture, true);
      window.removeEventListener("pointercancel", deferCancelledGesture, true);
      window.removeEventListener("blur", handoffOrCancelOnBlur);
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
    const handleFileDragKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        cancelFileDragGesture();
        return;
      }
      // macOS can consume Tab during app switching; Meta is the reliable early
      // signal that lets the native drag session exist before focus changes.
      if (event.key !== "Meta" && !(event.key === "Tab" && event.metaKey)) return;
      const gesture = fileDragGestureRef.current;
      if (!gesture) return;
      if (decideFileDragHandoff({
        phase: gesture.phase,
        fileCount: gesture.fileIds.length,
        desktop: isTauri(),
        trigger: "command",
      }) === "promote") {
        promoteToNativeFileDrag(gesture, true);
      }
    };
    window.addEventListener("keydown", handleFileDragKeyDown, true);
    return () => window.removeEventListener("keydown", handleFileDragKeyDown, true);
  });

  const stopMarqueeAutoScroll = () => {
    if (marqueeAutoScrollFrameRef.current === null) return;
    window.cancelAnimationFrame(marqueeAutoScrollFrameRef.current);
    marqueeAutoScrollFrameRef.current = null;
  };

  const updateMarqueeSelection = (clientX: number, clientY: number) => {
    const gesture = marqueeGestureRef.current;
    const scroll = contentDropZoneRef.current;
    const grid = fileGridRef.current;
    if (!gesture || !scroll || !grid || !fileJustifiedLayout) return;
    gesture.lastClientX = clientX;
    gesture.lastClientY = clientY;
    if (
      gesture.phase === "pending"
      && Math.hypot(clientX - gesture.startClientX, clientY - gesture.startClientY) < fileMarqueeThreshold
    ) return;

    gesture.phase = "selecting";
    const scrollRect = scroll.getBoundingClientRect();
    const end = pointInScrollContent(clientX, clientY, {
      left: scrollRect.left,
      top: scrollRect.top,
      scrollLeft: scroll.scrollLeft,
      scrollTop: scroll.scrollTop,
      scrollWidth: scroll.scrollWidth,
      scrollHeight: scroll.scrollHeight,
    });
    const rectangle = normalizeSelectionRectangle(gesture.startX, gesture.startY, end.x, end.y);
    const intersectingIds = new Set<string>();
    const gridRect = grid.getBoundingClientRect();
    const gridLeft = gridRect.left - scrollRect.left + scroll.scrollLeft;
    const gridTop = gridRect.top - scrollRect.top + scroll.scrollTop;
    for (const file of visibleFiles) {
      const placement = fileJustifiedLayout.placements[file.id];
      if (!placement) continue;
      const cardTop = gridTop + placement.y;
      if (cardTop > rectangle.bottom) break;
      const cardBottom = cardTop + fileJustifiedLayout.cardHeight;
      if (cardBottom < rectangle.top) continue;
      if (rectanglesIntersect(rectangle, {
        left: gridLeft + placement.x,
        top: cardTop,
        right: gridLeft + placement.x + placement.width,
        bottom: cardBottom,
      })) intersectingIds.add(file.id);
    }
    updateSelectedFiles(combineMarqueeSelection(gesture.baselineIds, intersectingIds, gesture.mode));
    setSelectionMarquee({
      left: rectangle.left,
      top: rectangle.top,
      width: rectangle.right - rectangle.left,
      height: rectangle.bottom - rectangle.top,
    });
  };

  const marqueeScrollSpeed = (clientY: number) => {
    const scroll = contentDropZoneRef.current;
    if (!scroll) return 0;
    const rect = scroll.getBoundingClientRect();
    if (clientY < rect.top + fileMarqueeScrollInset) {
      const intensity = Math.min(1, (rect.top + fileMarqueeScrollInset - clientY) / fileMarqueeScrollInset);
      return -Math.max(2, intensity * fileMarqueeMaxScrollSpeed);
    }
    if (clientY > rect.bottom - fileMarqueeScrollInset) {
      const intensity = Math.min(1, (clientY - (rect.bottom - fileMarqueeScrollInset)) / fileMarqueeScrollInset);
      return Math.max(2, intensity * fileMarqueeMaxScrollSpeed);
    }
    return 0;
  };

  const ensureMarqueeAutoScroll = () => {
    if (marqueeAutoScrollFrameRef.current !== null) return;
    const tick = () => {
      marqueeAutoScrollFrameRef.current = null;
      const gesture = marqueeGestureRef.current;
      const scroll = contentDropZoneRef.current;
      if (!gesture || gesture.phase !== "selecting" || !scroll) return;
      const speed = marqueeScrollSpeed(gesture.lastClientY);
      if (speed === 0) return;
      const before = scroll.scrollTop;
      scroll.scrollTop += speed;
      if (scroll.scrollTop === before) return;
      updateMarqueeSelection(gesture.lastClientX, gesture.lastClientY);
      marqueeAutoScrollFrameRef.current = window.requestAnimationFrame(tick);
    };
    marqueeAutoScrollFrameRef.current = window.requestAnimationFrame(tick);
  };

  const beginMarqueeSelection = (event: React.PointerEvent<HTMLDivElement>) => {
    const target = event.target instanceof Element ? event.target : null;
    if (
      event.button !== 0
      || !event.isPrimary
      || target?.closest(".file-space-file-card, .file-space-folder-grid > button, button, input, textarea, select, [contenteditable='true']")
      || operationRef.current
      || internalFileDragRef.current
      || internalFolderDragRef.current
      || isFileSpaceDialogMounted
    ) return;
    const scrollRect = event.currentTarget.getBoundingClientRect();
    const innerLeft = scrollRect.left + event.currentTarget.clientLeft;
    const innerTop = scrollRect.top + event.currentTarget.clientTop;
    const usesOverlayVerticalScrollbar = event.currentTarget.scrollHeight > event.currentTarget.clientHeight
      && event.currentTarget.offsetWidth - event.currentTarget.clientWidth <= event.currentTarget.clientLeft * 2;
    const usesOverlayHorizontalScrollbar = event.currentTarget.scrollWidth > event.currentTarget.clientWidth
      && event.currentTarget.offsetHeight - event.currentTarget.clientHeight <= event.currentTarget.clientTop * 2;
    const innerRight = innerLeft + event.currentTarget.clientWidth - (usesOverlayVerticalScrollbar ? 13 : 0);
    const innerBottom = innerTop + event.currentTarget.clientHeight - (usesOverlayHorizontalScrollbar ? 13 : 0);
    if (
      event.clientX < innerLeft
      || event.clientX >= innerRight
      || event.clientY < innerTop
      || event.clientY >= innerBottom
    ) return;
    const mode: FileSelectionMode = event.shiftKey
      ? "union"
      : event.metaKey || event.ctrlKey
      ? "toggle"
      : "replace";
    const start = pointInScrollContent(event.clientX, event.clientY, {
      left: scrollRect.left,
      top: scrollRect.top,
      scrollLeft: event.currentTarget.scrollLeft,
      scrollTop: event.currentTarget.scrollTop,
      scrollWidth: event.currentTarget.scrollWidth,
      scrollHeight: event.currentTarget.scrollHeight,
    });
    marqueeGestureRef.current = {
      pointerId: event.pointerId,
      startX: start.x,
      startY: start.y,
      startClientX: event.clientX,
      startClientY: event.clientY,
      lastClientX: event.clientX,
      lastClientY: event.clientY,
      baselineIds: new Set(selectedFileIdsRef.current),
      baselineAnchorId: selectionAnchorRef.current,
      mode,
      phase: "pending",
    };
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Synthetic accessibility/testing events may not create a native capture session.
    }
    event.currentTarget.focus({ preventScroll: true });
    event.preventDefault();
  };

  const continueMarqueeSelection = (event: React.PointerEvent<HTMLDivElement>) => {
    const gesture = marqueeGestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    if ((event.buttons & 1) === 0) {
      finishMarqueeSelection(event.pointerId);
      return;
    }
    event.preventDefault();
    updateMarqueeSelection(event.clientX, event.clientY);
    if (gesture.phase === "selecting") ensureMarqueeAutoScroll();
  };

  const finishMarqueeSelection = (pointerId: number, restoreBaseline = false) => {
    const gesture = marqueeGestureRef.current;
    const scroll = contentDropZoneRef.current;
    if (!gesture || gesture.pointerId !== pointerId) return;
    if (restoreBaseline) {
      updateSelectedFiles(new Set(gesture.baselineIds), gesture.baselineAnchorId);
    } else if (gesture.phase === "pending" && gesture.mode === "replace") {
      clearSelectedFiles();
    }
    if (!restoreBaseline && gesture.phase === "selecting") {
      const preservedAnchor = gesture.mode !== "replace"
        && gesture.baselineAnchorId
        && selectedFileIdsRef.current.has(gesture.baselineAnchorId)
        ? gesture.baselineAnchorId
        : null;
      selectionAnchorRef.current = preservedAnchor ?? sortedVisibleFiles.find((file) => (
        selectedFileIdsRef.current.has(file.id)
      ))?.id ?? null;
      setSelectionAnnouncement(t("fileSpace.inspector.selectionStatus", {
        count: selectedFileIdsRef.current.size,
      }));
    }
    marqueeGestureRef.current = null;
    stopMarqueeAutoScroll();
    setSelectionMarquee(null);
    if (scroll?.hasPointerCapture(pointerId)) scroll.releasePointerCapture(pointerId);
  };

  const cancelActiveMarquee = () => {
    const gesture = marqueeGestureRef.current;
    if (gesture) finishMarqueeSelection(gesture.pointerId, true);
  };

  const focusFileCard = (fileId: string) => {
    const scroll = contentDropZoneRef.current;
    const grid = fileGridRef.current;
    const placement = fileJustifiedLayout?.placements[fileId];
    if (!scroll || !grid || !placement || !fileJustifiedLayout) return;
    const scrollRect = scroll.getBoundingClientRect();
    const gridRect = grid.getBoundingClientRect();
    const targetTop = gridRect.top - scrollRect.top + scroll.scrollTop + placement.y;
    const targetBottom = targetTop + fileJustifiedLayout.cardHeight;
    const viewportTop = scroll.scrollTop;
    const viewportBottom = viewportTop + scroll.clientHeight;
    if (targetTop < viewportTop + 12) scroll.scrollTop = Math.max(0, targetTop - 12);
    else if (targetBottom > viewportBottom - 12) {
      scroll.scrollTop = Math.max(0, targetBottom - scroll.clientHeight + 12);
    }
    updateFileVirtualViewport();
    window.requestAnimationFrame(() => window.requestAnimationFrame(() => {
      document.querySelector<HTMLButtonElement>(
        `.file-space-file-card[data-file-id="${CSS.escape(fileId)}"] > button:first-child`,
      )?.focus({ preventScroll: true });
    }));
  };

  const handleFileGridKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const shortcutAction = fileKeyboardShortcutAction(event.nativeEvent);
    if (shortcutAction) {
      const target = event.target;
      const editingText = target instanceof Element
        && Boolean(target.closest("input, textarea, select, [contenteditable='true']"));
      const interactionBlocked = editingText
        || nativeDropBlocked
        || Boolean(operationRef.current)
        || Boolean(fileDragGestureRef.current)
        || Boolean(marqueeGestureRef.current)
        || Boolean(document.querySelector('[aria-modal="true"]'));
      if (interactionBlocked || selectedFileIdsRef.current.size !== 1) return;

      const [selectedFileId] = selectedFileIdsRef.current;
      const file = selectedFileId ? visibleFileById.get(selectedFileId) : null;
      if (!file) return;

      if (shortcutAction === "trash") {
        event.preventDefault();
        event.stopPropagation();
        requestDeleteFile(file.id);
        return;
      }

      const cardButton = document.querySelector<HTMLButtonElement>(
        `.file-space-file-card[data-file-id="${CSS.escape(file.id)}"] > button:first-child`,
      );
      if (!cardButton) return;
      const route = resolveFileDoubleClickRoute(
        file.name,
        Boolean(cardButton.querySelector(".file-space-file-art.has-preview")),
      );
      if (route !== "internal-preview") return;
      event.preventDefault();
      event.stopPropagation();
      cardButton.dispatchEvent(new MouseEvent("dblclick", {
        bubbles: true,
        cancelable: true,
        button: 0,
        view: window,
      }));
      return;
    }

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      const allIds = sortedVisibleFiles.map((file) => file.id);
      updateSelectedFiles(new Set(allIds), allIds[0] ?? null);
      setSelectionAnnouncement(t("fileSpace.inspector.selectionStatus", { count: allIds.length }));
      return;
    }
    const directionByKey: Partial<Record<string, FileKeyboardDirection>> = {
      ArrowLeft: "left",
      ArrowRight: "right",
      ArrowUp: "up",
      ArrowDown: "down",
    };
    const direction = directionByKey[event.key];
    if (direction && fileJustifiedLayout) {
      const focusedFileId = document.activeElement instanceof Element
        ? document.activeElement.closest<HTMLElement>(".file-space-file-card")?.dataset.fileId ?? null
        : null;
      const currentId = focusedFileId && visibleFileById.has(focusedFileId)
        ? focusedFileId
        : selectionAnchorRef.current && visibleFileById.has(selectionAnchorRef.current)
          ? selectionAnchorRef.current
          : visibleFileIds.find((id) => selectedFileIdsRef.current.has(id)) ?? null;
      const nextId = nextFileIdForKeyboard({
        orderedIds: visibleFileIds,
        placements: fileJustifiedLayout.placements,
        currentId,
        direction,
        layoutMode: fileLayoutMode,
      });
      if (nextId) {
        event.preventDefault();
        updateSelectedFiles(new Set([nextId]), nextId);
        focusFileCard(nextId);
      }
      return;
    }
    if (
      event.key === "Escape"
      && !marqueeGestureRef.current
      && !fileDragGestureRef.current
      && !internalFileDragRef.current
      && !internalFolderDragRef.current
      && selectedFileIdsRef.current.size > 0
    ) {
      event.preventDefault();
      clearSelectedFiles();
    }
  };

  useEffect(() => {
    document.body.classList.toggle("is-file-space-marquee-selecting", Boolean(selectionMarquee));
    return () => document.body.classList.remove("is-file-space-marquee-selecting");
  }, [selectionMarquee]);

  useEffect(() => {
    const cancelOnBlur = () => cancelActiveMarquee();
    const cancelOnResize = () => cancelActiveMarquee();
    const cancelWithEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && marqueeGestureRef.current) {
        event.preventDefault();
        event.stopPropagation();
        cancelActiveMarquee();
      }
    };
    window.addEventListener("blur", cancelOnBlur);
    window.addEventListener("resize", cancelOnResize);
    window.addEventListener("keydown", cancelWithEscape, true);
    return () => {
      window.removeEventListener("blur", cancelOnBlur);
      window.removeEventListener("resize", cancelOnResize);
      window.removeEventListener("keydown", cancelWithEscape, true);
    };
  });

  useEffect(() => {
    if (marqueeGestureRef.current) cancelActiveMarquee();
  }, [fileLayoutMode, visibleFileIds, previewSize]);

  const openRenameFile = (fileId: string) => {
    if (!canWrite) return;
    if (operationRef.current) return;
    const file = snapshot.files.find((item) => item.id === fileId);
    if (!file) return;
    rememberDialogReturnFocus(document.querySelector<HTMLElement>(`[data-file-id="${CSS.escape(fileId)}"] > button:first-child`));
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
    if (!canWrite) return;
    if (operationRef.current) return;
    rememberDialogReturnFocus(document.querySelector<HTMLElement>(`[data-file-id="${CSS.escape(fileId)}"] > button:first-child`));
    nativeDropBlockedRef.current = true;
    setFileContextMenu(null);
    setDeleteFileId(fileId);
  };

  const deleteFile = async () => {
    if (!deleteFileId) return;
    const fileName = snapshot.files.find((file) => file.id === deleteFileId)?.name ?? "";
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
      setTrashFeedback(t("fileSpace.trash.movedToTrash", { name: fileName }));
    } catch (deleteError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.deleteFile")} ${errorText(deleteError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const openRenameFolder = (folderId: string) => {
    if (!canWrite) return;
    if (operationRef.current) return;
    const folder = snapshot.folders.find((item) => item.id === folderId);
    if (!folder) return;
    rememberDialogReturnFocus(document.querySelector<HTMLElement>(`[data-folder-tree-id="${CSS.escape(folderId)}"] .file-space-tree-name`));
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
    if (!canWrite) return;
    if (operationRef.current) return;
    rememberDialogReturnFocus(document.querySelector<HTMLElement>(`[data-folder-tree-id="${CSS.escape(folderId)}"] .file-space-tree-name`));
    nativeDropBlockedRef.current = true;
    setFolderContextMenu(null);
    setDeleteFolderId(folderId);
  };

  const deleteFolder = async () => {
    if (!deleteFolderId) return;
    const folderNameToDelete = snapshot.folders.find((folder) => folder.id === deleteFolderId)?.name ?? "";
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
      setTrashFeedback(t("fileSpace.trash.movedToTrash", { name: folderNameToDelete }));
    } catch (deleteError) {
      if (canCommitOperation(operation)) {
        setError(`${t("fileSpace.errors.deleteFolder")} ${errorText(deleteError)}`);
      }
    } finally {
      finishOperation(operation);
    }
  };

  const requestRestoreTrashEntry = (entryId: string, focusEntryId: string | null) => {
    if (operationRef.current) return;
    const fallback = focusEntryId
      ? document.querySelector<HTMLElement>(
          `[data-trash-entry-restore="${CSS.escape(focusEntryId)}"]`,
        )
      : document.querySelector<HTMLElement>("[data-trash-heading]");
    rememberDialogReturnFocus(fallback);
    nativeDropBlockedRef.current = true;
    setRestoreConfirmationEntryId(entryId);
  };

  const restoreTrashEntry = async () => {
    const entryId = restoreConfirmationEntryId;
    if (!entryId) return;
    const item = snapshot.trashItems.find((candidate) => candidate.id === entryId);
    if (!item) {
      setRestoreConfirmationEntryId(null);
      return;
    }
    const operation = beginOperation("restore");
    if (!operation) return;
    setRestoringTrashEntryId(entryId);
    setError(null);
    try {
      const updated = await invoke<FileSpaceSnapshot>("restore_file_space_trash_entry", { entryId });
      if (!canCommitOperation(operation)) return;
      setSnapshot(updated);
      setRestoreConfirmationEntryId(null);
      const restored = item.itemType === "folder"
        ? updated.folders.find((folder) => folder.id === item.rootId)
        : updated.files.find((file) => file.id === item.rootId);
      setTrashFeedback(t("fileSpace.trash.restored", { name: restored?.name ?? item.name }));
    } catch (restoreError) {
      if (canCommitOperation(operation)) {
        setRestoreConfirmationEntryId(null);
        setError(`${t("fileSpace.errors.restoreTrash")} ${errorText(restoreError)}`);
      }
    } finally {
      if (canCommitOperation(operation)) setRestoringTrashEntryId(null);
      finishOperation(operation);
    }
  };

  const requestEmptyTrash = (trigger: HTMLElement) => {
    if (operationRef.current || snapshot.trashItems.length === 0) return;
    setEmptyTrashEntryIds(snapshot.trashItems.map((item) => item.id));
    rememberDialogReturnFocus(
      document.querySelector<HTMLElement>("[data-trash-heading]"),
    );
    dialogReturnFocusRef.current = trigger;
    nativeDropBlockedRef.current = true;
    setEmptyTrashConfirmationOpen(true);
  };

  const emptyTrash = async () => {
    const entryIds = [...emptyTrashEntryIds];
    if (!isEmptyTrashConfirmationOpen || entryIds.length === 0) return;
    const operation = beginOperation("purgeTrash");
    if (!operation) return;
    setError(null);
    try {
      const result = await invoke<FileSpaceTrashPurgeResult>("empty_file_space_trash", { entryIds });
      if (!canCommitOperation(operation)) return;
      setSnapshot(result.snapshot);
      if (result.snapshot.trashItems.length === 0) dialogReturnFocusRef.current = null;
      setEmptyTrashConfirmationOpen(false);
      if (result.failedCount > 0 || result.failureMessage) {
        const summary = t("fileSpace.trash.partialPurge", {
          failed: result.failedCount,
          purged: result.purgedCount,
        });
        setError(`${t("fileSpace.errors.emptyTrash")} ${summary}${result.failureMessage ? ` ${result.failureMessage}` : ""}`);
      } else {
        setTrashFeedback(t("fileSpace.trash.emptied"));
      }
    } catch (emptyError) {
      if (canCommitOperation(operation)) {
        setEmptyTrashConfirmationOpen(false);
        setError(`${t("fileSpace.errors.emptyTrash")} ${errorText(emptyError)}`);
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
    const sidebarScroll = sidebarScrollRef.current;
    if (sidebarScroll) {
      const rect = sidebarScroll.getBoundingClientRect();
      if (event.clientY < rect.top + 26) sidebarScroll.scrollTop -= 9;
      else if (event.clientY > rect.bottom - 26) sidebarScroll.scrollTop += 9;
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
    aiSourceNavigationSequenceRef.current += 1;
    fileRevealNavigationRef.current.cancel();
    setActiveCollection("files");
    setCurrentFolderId(folderId);
    clearSelectedFiles();
    setQuery("");
    if (window.matchMedia("(max-width: 980px)").matches) setSidebarVisible(false);
  };

  const chooseTrash = () => {
    aiSourceNavigationSequenceRef.current += 1;
    fileRevealNavigationRef.current.cancel();
    setActiveCollection("trash");
    clearSelectedFiles();
    setQuery("");
    setFilterMenuOpen(false);
    setFolderContextMenu(null);
    setFileContextMenu(null);
    closeTimelinePanel();
    if (window.matchMedia("(max-width: 980px)").matches) setSidebarVisible(false);
  };

  const toggleExpanded = (folderId: string) => {
    setExpandedFolders((current) => {
      const next = new Set(current);
      if (next.has(folderId)) next.delete(folderId);
      else next.add(folderId);
      return next;
    });
  };

  const toggleFolderSubtree = (folderId: string) => {
    const expandableIds = expandableFolderIdsInSubtree(snapshot.folders, folderId);
    setExpandedFolders((current) => setFolderSubtreeExpanded(
      current,
      expandableIds,
      !isFolderSubtreeFullyExpanded(current, expandableIds),
    ));
    setFolderContextMenu(null);
  };

  const visibleFolderTreeIds = useMemo(() => {
    const ids: string[] = [];
    const collect = (parentId: string | null) => {
      (foldersByParent.get(parentId) ?? []).forEach((folder) => {
        ids.push(folder.id);
        if (expandedFolders.has(folder.id)) collect(folder.id);
      });
    };
    collect(null);
    return ids;
  }, [expandedFolders, foldersByParent]);
  const folderTreeTabStopId = currentFolder?.id && visibleFolderTreeIds.includes(currentFolder.id)
    ? currentFolder.id
    : visibleFolderTreeIds[0] ?? null;

  const focusFolderTreeItem = (folderId: string) => {
    const item = folderTreeRef.current?.querySelector<HTMLElement>(
      `.file-space-tree-name[data-folder-id="${CSS.escape(folderId)}"]`,
    );
    item?.focus({ preventScroll: true });
    item?.scrollIntoView({ block: "nearest" });
  };

  const handleFolderTreeKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!(event.target instanceof Element)) return;
    const item = event.target.closest<HTMLElement>(".file-space-tree-name[data-folder-id]");
    const folderId = item?.dataset.folderId;
    if (!folderId) return;
    const index = visibleFolderTreeIds.indexOf(folderId);
    if (index < 0) return;
    if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      event.stopPropagation();
    } else return;

    if (event.key === "ArrowUp") focusFolderTreeItem(visibleFolderTreeIds[Math.max(0, index - 1)]);
    else if (event.key === "ArrowDown") focusFolderTreeItem(visibleFolderTreeIds[Math.min(visibleFolderTreeIds.length - 1, index + 1)]);
    else if (event.key === "Home") focusFolderTreeItem(visibleFolderTreeIds[0]);
    else if (event.key === "End") focusFolderTreeItem(visibleFolderTreeIds.at(-1)!);
    else if (event.key === "ArrowRight") {
      const firstChild = (foldersByParent.get(folderId) ?? [])[0];
      if (!firstChild) return;
      if (!expandedFolders.has(folderId)) toggleExpanded(folderId);
      else focusFolderTreeItem(firstChild.id);
    } else if (expandedFolders.has(folderId) && (foldersByParent.get(folderId) ?? []).length > 0) {
      toggleExpanded(folderId);
    } else {
      const parentId = snapshot.folders.find((folder) => folder.id === folderId)?.parentId;
      if (parentId) focusFolderTreeItem(parentId);
    }
  };

  const renderTree = (parentId: string | null, depth = 0): React.ReactNode => (
    (foldersByParent.get(parentId) ?? []).map((folder) => {
      const hasChildren = (foldersByParent.get(folder.id) ?? []).length > 0;
      const expanded = expandedFolders.has(folder.id);
      const active = activeCollection === "files" && currentFolder?.id === folder.id;
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
              tabIndex={-1}
              disabled={!hasChildren}
              onClick={() => toggleExpanded(folder.id)}
              style={{ left: `${depth * 17 - 15}px` }}
              aria-label={hasChildren ? folder.name : undefined}
              aria-expanded={hasChildren ? expanded : undefined}
            >
              {hasChildren ? <ChevronRight className="file-space-tree-chevron" size={13} /> : null}
            </button>
            <button
              className="file-space-tree-name"
              type="button"
              role="treeitem"
              data-folder-id={folder.id}
              tabIndex={folderTreeTabStopId === folder.id ? 0 : -1}
              aria-level={depth + 1}
              aria-selected={active}
              aria-expanded={hasChildren ? expanded : undefined}
              aria-grabbed={internalFolderDrag?.folderId === folder.id}
              onPointerDown={(event) => { if (canWrite) beginFolderDragGesture(event, folder.id); }}
              onClick={() => {
                if (suppressFolderClickRef.current) {
                  suppressFolderClickRef.current = false;
                  return;
                }
                chooseFolder(folder.id);
              }}
              onDoubleClick={(event) => {
                event.preventDefault();
                setExpandedFolders((current) => (
                  toggleFolderOnDoubleClick(current, folder.id, hasChildren)
                ));
              }}
              title={folder.name}
            >
              {active ? <FolderOpen size={15} /> : <Folder size={15} />}
              <span>{folder.name}</span>
              <small>{folderCounts.get(folder.id) ?? 0}</small>
            </button>
          </div>
          {hasChildren ? (
            <FolderTreeChildren
              expanded={expanded}
              renderChildren={() => renderTree(folder.id, depth + 1)}
            />
          ) : null}
        </div>
      );
    })
  );

  const rootView = fileSpaceRootView(loading, initialLoadError, snapshot.rootStatus);

  useEffect(() => {
    if (rootView === "loading") return undefined;
    return dismissStartupSplash();
  }, [rootView]);

  const workspaceControls = <>
    <FileSpaceWorkspaceSwitcher directory={workspaceDirectory} disabled={Boolean(busyAction)} onWorkspaceChanged={applyWorkspaceMutation} />
    <FileSpaceSettingsMenu<FileSpaceSnapshot>
      workspaceDirectory={workspaceDirectory}
      workspaceDisabled={Boolean(busyAction)}
      onWorkspaceChanged={applyWorkspaceMutation}
      onWorkspaceDirectoryChanged={setWorkspaceDirectory}
    />
  </>;
  if (rootView === "loading") {
    return (
      <section className="file-space-page file-space-page--loading" aria-busy="true">
        <div className="file-space-setup-titlebar" data-tauri-drag-region aria-hidden="true" />
        <div className="file-space-loading-state" role="status" aria-live="polite">
          <span className="file-space-loading-mark" aria-hidden="true" />
          <strong>{t("fileSpace.root.opening")}</strong>
          <span>{t("fileSpace.root.openingDescription")}</span>
        </div>
      </section>
    );
  }

  if (rootView === "loadError" && initialLoadError) {
    return (
      <section className="file-space-page file-space-page--loading">
        <footer className="file-space-recovery-workspaces file-space-sidebar-footer">{workspaceControls}</footer>
        <div className="file-space-setup-titlebar" data-tauri-drag-region aria-hidden="true" />
        <div className="file-space-empty" role="alert" aria-live="assertive">
          <span className="file-space-empty-symbol" aria-hidden="true">
            <CircleAlert size={24} strokeWidth={1.7} />
          </span>
          <strong>{t("fileSpace.root.loadFailedTitle")}</strong>
          <span>{t("fileSpace.root.loadFailedDescription")}</span>
          <p className="file-space-error">{initialLoadError}</p>
          <div className="file-space-empty-actions">
            <button className="is-primary" type="button" onClick={() => void loadSnapshot()}>
              {t("fileSpace.root.retryLoad")}
            </button>
          </div>
        </div>
      </section>
    );
  }

  if (rootView === "setup" && snapshot.rootStatus !== "ready") {
    const rootStatusMessage = snapshot.rootStatus === "unconfigured"
      ? null
      : t(`fileSpace.root.status.${snapshot.rootStatus}`);
    return (
      <section className="file-space-page file-space-setup-page">
        <footer className="file-space-recovery-workspaces file-space-sidebar-footer">{workspaceControls}</footer>
        <div className="file-space-setup-titlebar" data-tauri-drag-region aria-hidden="true" />
        <SetupPreferences />
        <div className="file-space-setup">
          <img
            className="file-space-setup-logo"
            src={lumeTraceLogo}
            alt=""
            aria-hidden="true"
            draggable={false}
          />
          <p className="eyebrow">Lume Trace · {t("fileSpace.eyebrow")}</p>
          <h1>{t("fileSpace.root.title")}</h1>
          <p className="file-space-setup-description">{t("fileSpace.root.description")}</p>
          {snapshot.rootPath ? (
            <div className="file-space-invalid-path">
              <span className="file-space-invalid-path-icon" aria-hidden="true"><FolderOpen size={18} /></span>
              <span className="file-space-invalid-path-copy">
                <strong>{rootStatusMessage}</strong>
                <span>{snapshot.rootPath}</span>
              </span>
            </div>
          ) : null}
          <div className="file-space-setup-options">
            <button
              className="file-space-setup-option"
              type="button"
              disabled={busyAction === "configure"}
              aria-busy={busyAction === "configure" && rootSetupMode === "new"}
              onClick={() => void chooseStorageRoot("new")}
            >
              <span className="file-space-setup-option-icon" aria-hidden="true">
                {busyAction === "configure" && rootSetupMode === "new"
                  ? <LoaderCircle className="is-spinning" size={19} />
                  : <FolderPlus size={19} />}
              </span>
              <span className="file-space-setup-option-copy">
                <strong>{busyAction === "configure" && rootSetupMode === "new"
                  ? t("fileSpace.root.choosingNew")
                  : t("fileSpace.root.createNew")}</strong>
                <small>{t("fileSpace.root.createNewDescription")}</small>
              </span>
              <ChevronRight size={16} aria-hidden="true" />
            </button>
            <button
              className="file-space-setup-option"
              type="button"
              disabled={busyAction === "configure"}
              aria-busy={busyAction === "configure" && rootSetupMode === "import"}
              onClick={(event) => void chooseStorageRoot("import", event.currentTarget)}
            >
              <span className="file-space-setup-option-icon" aria-hidden="true">
                {busyAction === "configure" && rootSetupMode === "import"
                  ? <LoaderCircle className="is-spinning" size={19} />
                  : <Upload size={19} />}
              </span>
              <span className="file-space-setup-option-copy">
                <strong>{busyAction === "configure" && rootSetupMode === "import"
                  ? t("fileSpace.root.importingExisting")
                  : t("fileSpace.root.importExisting")}</strong>
                <small>{t("fileSpace.root.importExistingDescription")}</small>
              </span>
              <ChevronRight size={16} aria-hidden="true" />
            </button>
          </div>
          <ApplicationExtensionEntry placement="setup" disabled={busyAction === "configure"} />
          {setupCancelled ? <p className="file-space-setup-feedback">{t("fileSpace.root.cancel")}</p> : null}
          {error ? <p className="file-space-error" role="alert">{error}</p> : null}
          <p className="file-space-setup-privacy">
            <ShieldCheck size={14} />
            <span>{t("fileSpace.root.privacy")}</span>
          </p>
        </div>
        <ImportExistingFolderSheet
          path={pendingImportPath}
          busy={busyAction === "configure"}
          progress={importFeedback && importFeedback.requestId === importRequestRef.current
            ? { processed: importFeedback.processed, total: importFeedback.total }
            : null}
          error={error}
          returnFocusTarget={importExistingTriggerRef.current}
          onCancel={() => { setPendingImportPath(null); setError(null); }}
          onChooseAgain={() => void chooseStorageRoot("import")}
          onConfirm={() => void confirmExistingFolderImport()}
        />
      </section>
    );
  }

  return (
    <section className={`file-space-page${isSidebarVisible ? "" : " is-sidebar-hidden"}${isInspectorVisible && !isTrashView ? "" : " is-inspector-hidden"}`}>
      <aside ref={sidebarRef} className="file-space-sidebar" aria-hidden={!isSidebarVisible} inert={!isSidebarVisible}>
        <header className="file-space-sidebar-header" data-tauri-drag-region>
          <button
            className="file-space-sidebar-close"
            type="button"
            onClick={() => setSidebarVisible(false)}
            title={t("fileSpace.toolbar.toggleSidebar")}
            aria-label={t("fileSpace.toolbar.toggleSidebar")}
          >
            <X size={17} />
          </button>
        </header>

        <div className="file-space-sidebar-scroll" ref={sidebarScrollRef}>
          <div
            id="file-space-favorites-heading"
            className="file-space-folder-heading file-space-favorites-heading"
            role="heading"
            aria-level={2}
          >
            <span>{t("fileSpace.sidebar.favorites")}</span>
          </div>
          <nav className="file-space-quick-navigation" aria-labelledby="file-space-favorites-heading">
            <button
              className={`${activeCollection === "files" && !currentFolder ? "is-active" : ""}${internalFileDrag?.folderTarget?.kind === "root" ? (internalFileDrag.folderTarget.isCurrent ? " is-file-drop-current" : " is-file-drop-target") : ""}`}
              type="button"
              data-file-root-drop="true"
              aria-current={activeCollection === "files" && !currentFolder ? "page" : undefined}
              onClick={() => chooseFolder(null)}
            >
              <Folder size={16} /><span>{t("fileSpace.sidebar.all")}</span><small>{snapshot.fileCount}</small>
            </button>
            <button
              className={isTrashView ? "is-active" : ""}
              type="button"
              aria-current={isTrashView ? "page" : undefined}
              onClick={chooseTrash}
            >
              <Trash2 size={16} /><span>{t("fileSpace.sidebar.trash")}</span><small>{snapshot.trashItems.length}</small>
            </button>
          </nav>

          <div
            className={`file-space-folder-heading${internalFolderDrag?.target?.mode === "root" ? " is-folder-root-drop" : ""}`}
            data-folder-root-drop="true"
          >
            <span>{t("fileSpace.sidebar.folders")}</span>
            <button type="button" disabled={!canWrite || Boolean(busyAction)} onClick={() => openCreateFolder(null)} aria-label={t("fileSpace.sidebar.addFolder")}>
              <Plus size={15} />
            </button>
          </div>
          <div
            className={`file-space-tree${internalFolderDrag?.target?.mode === "root" ? " is-folder-root-drop" : ""}`}
            ref={folderTreeRef}
            role="tree"
            data-folder-root-drop="true"
            onKeyDown={handleFolderTreeKeyDown}
          >
            {renderTree(null)}
          </div>
        </div>

        <footer className="file-space-sidebar-footer">
          {workspaceControls}
        </footer>
      </aside>

      <main className="file-space-workspace">
        {externalWorkspace?.notice ? <div className="file-space-source-notice" role="status">{externalWorkspace.notice}</div> : null}
        <header className="file-space-workspace-toolbar" data-tauri-drag-region>
          <div className="file-space-toolbar-navigation">
            <button
              className="file-space-toolbar-icon-button"
              type="button"
              data-file-space-sidebar-toggle="true"
              onClick={toggleSidebarVisibility}
              title={t("fileSpace.toolbar.toggleSidebar")}
              aria-label={t("fileSpace.toolbar.toggleSidebar")}
              aria-pressed={isSidebarVisible}
            >
              <PanelLeft size={18} />
            </button>
            {currentFolder && !isTrashView ? (
              <>
                <button
                  className="file-space-toolbar-icon-button"
                  type="button"
                  onClick={() => chooseFolder(currentFolder.parentId ?? null)}
                  title={t("fileSpace.toolbar.back")}
                  aria-label={t("fileSpace.toolbar.back")}
                >
                  <ChevronLeft size={19} />
                </button>
                <strong className="file-space-toolbar-title" title={workspaceTitle} data-tauri-drag-region>{workspaceTitle}</strong>
              </>
            ) : isTrashView ? (
              <strong className="file-space-toolbar-title" title={workspaceTitle} data-trash-heading tabIndex={-1} data-tauri-drag-region>{workspaceTitle}</strong>
            ) : (
              <strong
                className="file-space-toolbar-title"
                title={t("fileSpace.sidebar.all")}
                data-tauri-drag-region
              >
                {t("fileSpace.sidebar.all")}
              </strong>
            )}
          </div>
          {isTrashView ? <>
            <span className="file-space-toolbar-spacer" />
            <button
              className="file-space-empty-trash-button"
              type="button"
              disabled={snapshot.trashItems.length === 0 || Boolean(busyAction)}
              onClick={(event) => requestEmptyTrash(event.currentTarget)}
            >
              {busyAction === "purgeTrash"
                ? t("fileSpace.trash.cleaning")
                : t("fileSpace.trash.emptyAction")}
            </button>
          </> : <><div className={`file-space-preview-size-control${fileLayoutMode === "list" ? " is-disabled" : ""}`}>
            <button
              type="button"
              disabled={fileLayoutMode === "list" || previewSize <= previewSizeMin}
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
              disabled={fileLayoutMode === "list"}
              onInput={(event) => updatePreviewSize(Number(event.currentTarget.value))}
              aria-label={t("fileSpace.toolbar.previewSize")}
              style={{ "--preview-size-progress": `${((previewSize - previewSizeMin) / (previewSizeMax - previewSizeMin)) * 100}%` } as React.CSSProperties}
            />
            <button
              type="button"
              disabled={fileLayoutMode === "list" || previewSize >= previewSizeMax}
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
          <button
            className="file-space-search file-space-search-trigger"
            type="button"
            disabled={!canSearch}
            onClick={openGlobalSearch}
            title={t("fileSpace.globalSearch.openShortcut", { shortcut: globalSearchShortcut })}
            aria-label={t("fileSpace.globalSearch.openShortcut", { shortcut: globalSearchShortcut })}
            aria-keyshortcuts={globalSearchShortcut === "⌘K" ? "Meta+K" : "Alt+K"}
          >
            <Search size={18} />
            <span>{t("fileSpace.globalSearch.trigger")}</span>
            <kbd className="file-space-search-shortcut">{globalSearchShortcut}</kbd>
          </button>
          {capabilities?.search !== false ? <FileSpaceBackgroundStatusButton /> : null}
          {capabilities?.ai !== false ? <FileSpaceAiSurface key={workspaceGeneration} onOpenSource={openFileFromAiSource} /> : null}
          <button
            className={`file-space-toolbar-icon-button${isInspectorVisible ? " is-active" : ""}`}
            type="button"
            onClick={toggleInspectorVisibility}
            title={t("fileSpace.toolbar.toggleInspector")}
            aria-label={t("fileSpace.toolbar.toggleInspector")}
            aria-pressed={isInspectorVisible}
          >
            <Info size={18} />
          </button>
          </>}
        </header>

        {!isTrashView && fileLayoutMode === "list" && (visibleFolders.length > 0 || visibleFiles.length > 0) ? (
          <div className="file-space-file-list-header" aria-hidden="true">
            <span />
            <span className="file-space-file-list-date-heading">
              {t("fileSpace.content.listColumns.name")}
              {sortOption === "nameAsc" || sortOption === "nameDesc" ? (
                <i>{sortOption === "nameAsc" ? "↑" : "↓"}</i>
              ) : null}
            </span>
            <span>{t("fileSpace.content.listColumns.versions")}</span>
            <span className="file-space-file-list-date-heading">
              {t("fileSpace.content.listColumns.extension")}
              {sortOption === "typeAsc" ? <i>↑</i> : null}
            </span>
            <span className="file-space-file-list-date-heading">
              {t("fileSpace.content.listColumns.fileSize")}
              {sortOption === "sizeDesc" ? <i>↓</i> : null}
            </span>
            <span className="file-space-file-list-date-heading">
              {t("fileSpace.content.listColumns.addedAt")}
              {sortOption === "updatedAsc" || sortOption === "updatedDesc" ? (
                <i>{sortOption === "updatedAsc" ? "↑" : "↓"}</i>
              ) : null}
            </span>
          </div>
        ) : null}

        {isTrashView ? (
          <div className="file-space-trash-scroll">
            <FileSpaceTrashView
              items={snapshot.trashItems}
              restoringId={restoringTrashEntryId}
              busy={Boolean(busyAction)}
              locale={locale}
              retentionMs={trashRetentionMs}
              formatFileSize={formatFileSize}
              onRestore={requestRestoreTrashEntry}
            />
          </div>
        ) : <div
          className={`file-space-content-scroll${fileLayoutMode === "list" ? " is-list-layout" : ""}${visibleFolders.length === 0 && filePageTotal === 0 && !filePageLoading ? " is-empty" : ""}${isFileDragOver ? " is-drag-over" : ""}${selectionMarquee ? " is-marquee-selecting" : ""}`}
          ref={contentDropZoneRef}
          tabIndex={-1}
          aria-busy={busyAction === "import"}
          onKeyDown={handleFileGridKeyDown}
          onDragEnter={handleExternalDragEnter}
          onDragOver={handleExternalDragOver}
          onDragLeave={handleExternalDragLeave}
          onDrop={handleExternalDrop}
          onPointerDown={beginMarqueeSelection}
          onPointerMove={continueMarqueeSelection}
          onPointerUp={(event) => finishMarqueeSelection(event.pointerId)}
          onPointerCancel={(event) => finishMarqueeSelection(event.pointerId, true)}
          onContextMenu={openContentContextMenu}
          onLostPointerCapture={(event) => {
            if (marqueeGestureRef.current?.pointerId === event.pointerId) {
              finishMarqueeSelection(event.pointerId, true);
            }
          }}
          onScroll={() => {
            scheduleFileVirtualViewportUpdate();
            const gesture = marqueeGestureRef.current;
            if (gesture?.phase === "selecting") {
              updateMarqueeSelection(gesture.lastClientX, gesture.lastClientY);
            }
          }}
        >
          {visibleFolders.length > 0 ? (
            <section className="file-space-content-section">
              <div className="file-space-folder-grid">
                {visibleFolders.map((folder) => {
                  const fileDropTarget = internalFileDrag?.folderTarget?.targetId === folder.id
                    ? internalFileDrag.folderTarget
                    : null;
                  return (
                  <button
                    className={fileDropTarget ? (fileDropTarget.isCurrent ? "is-file-drop-current" : "is-file-drop-target") : undefined}
                    data-file-folder-drop={folder.id}
                    key={folder.id}
                    type="button"
                    onDoubleClick={() => chooseFolder(folder.id)}
                    onClick={() => chooseFolder(folder.id)}
                    onContextMenu={(event) => openFolderContextMenu(event, folder.id)}
                  >
                    {fileLayoutMode === "list" ? (
                      <span className="file-space-folder-list-icon"><Folder size={22} strokeWidth={1.45} /></span>
                    ) : (
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
                    )}
                    {fileLayoutMode === "list" ? (
                      <>
                        <span className="file-space-file-list-name">
                          <strong>{folder.name}</strong>
                          <small>{t("fileSpace.content.fileCount", { count: folderDisplayDetails.get(folder.id)?.fileCount ?? 0 })}</small>
                        </span>
                        <span className="file-space-file-list-value">–</span>
                        <span className="file-space-file-list-value">–</span>
                        <span className="file-space-file-list-value">–</span>
                        <span className="file-space-file-list-value">{fileListDateFormatter.format(folder.createdAt)}</span>
                      </>
                    ) : (
                      <span className="file-space-item-copy">
                        <strong>{folder.name}</strong>
                        <small>
                          {t("fileSpace.content.fileCount", { count: folderDisplayDetails.get(folder.id)?.fileCount ?? 0 })}
                          {(folderDisplayDetails.get(folder.id)?.folderCount ?? 0) > 0
                            ? ` · ${t("fileSpace.content.subfolderCount", { count: folderDisplayDetails.get(folder.id)?.folderCount ?? 0 })}`
                            : null}
                        </small>
                      </span>
                    )}
                  </button>
                  );
                })}
              </div>
            </section>
          ) : null}

          {visibleFiles.length > 0 ? (
            <section className="file-space-content-section">
              <div
                className={`file-space-file-grid${fileLayoutMode === "list" ? " is-list-layout" : ""}`}
                ref={fileGridRef}
                role="group"
                aria-label={t("fileSpace.content.files")}
                style={{
                  "--file-preview-size": `${fileLayoutMode === "list" ? fileListPreviewSize : previewSize}px`,
                  height: fileJustifiedLayout ? `${fileJustifiedLayout.height}px` : undefined,
                } as React.CSSProperties}
              >
                {renderedFiles.map((file) => {
                  const placement = fileJustifiedLayout?.placements[file.id];
                  const imageDimensions = imageDimensionsByFileVersion[imageDimensionsKey(file.id, file.updatedAt)];
                  const extension = fileExtension(file);
                  return (
                    <div
                      className={`file-space-file-card${selectedFileIds.has(file.id) ? " is-selected" : ""}${internalFileDrag?.fileIds.includes(file.id) ? " is-reorder-placeholder" : ""}`}
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
                        aria-pressed={selectedFileIds.has(file.id)}
                        draggable={false}
                        onPointerDown={(event) => beginFileDragGesture(event, file.id)}
                        onClick={(event) => handleFileClick(event, file.id)}
                        onContextMenu={(event) => openFileContextMenu(event, file.id)}
                      >
                        <FileArtwork file={file} onImageDimensions={recordImageDimensions} showVersionBadge={false} />
                        {fileLayoutMode === "list" ? (
                          <>
                            <span className="file-space-file-list-name">
                              <strong>{file.name}</strong>
                            </span>
                            <span className="file-space-file-list-value">
                              {file.versionCount > 0
                                ? t("fileSpace.content.versionCount", { count: file.versionCount })
                                : "–"}
                            </span>
                            <span className="file-space-file-list-value">{extension || "–"}</span>
                            <span className="file-space-file-list-value">{formatFileSize(file.sizeBytes)}</span>
                            <span className="file-space-file-list-value">{fileListDateFormatter.format(file.createdAt)}</span>
                          </>
                        ) : (
                          <span className="file-space-item-copy">
                            <strong>{file.name}</strong>
                            <small><FileCardMetadata file={file} imageDimensions={imageDimensions} /></small>
                          </span>
                        )}
                      </button>
                      {fileLayoutMode !== "list" && shouldShowFileVersionBadge(file.versionCount) ? (
                        <VersionHistoryBadge
                          key={`${workspaceGeneration}:${file.id}:${file.updatedAt}:${file.versionCount}`}
                          name={file.name} count={file.versionCount}
                          busy={openingTimelineFileId === file.id}
                          disabled={Boolean(busyAction) || openingTimelineFileId !== null || isTimelinePanelOpen}
                          loadSummary={async () => {
                            if (!isTauri()) {
                              const fixture = createVisualTimeline(file)!;
                              return { ...fixture, workspaceId: "visual-preview", versions: fixture.versions.slice(0, 5) };
                            }
                            const workspaceId = workspaceDirectory?.currentWorkspaceId;
                            if (!workspaceId) throw new Error("The workspace is unavailable");
                            const summary = await invoke<VersionSummary>("get_file_version_summary", { workspaceId, fileId: file.id });
                            if (summary.workspaceId !== workspaceId || summary.fileId !== file.id) throw new Error("The workspace changed");
                            return summary;
                          }}
                          onContextMenu={(event) => openFileContextMenu(event, file.id)}
                          onOpen={async (anchor, versionId) => {
                            timelineBadgeReturnFocusRef.current = anchor;
                            if (await openTimeline(file, versionId)) {
                              window.requestAnimationFrame(() => {
                                timelinePanelRef.current?.querySelector<HTMLButtonElement>("header > button")?.focus({ preventScroll: true });
                              });
                            }
                          }}
                        />
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </section>
          ) : null}

          {selectionMarquee ? (
            <div
              className="file-space-selection-marquee"
              aria-hidden="true"
              style={{
                left: `${selectionMarquee.left}px`,
                top: `${selectionMarquee.top}px`,
                width: `${selectionMarquee.width}px`,
                height: `${selectionMarquee.height}px`,
              }}
            />
          ) : null}

          <p className="file-space-selection-status" role="status" aria-live="polite">{selectionAnnouncement}</p>

          {visibleFolders.length === 0 && filePageTotal === 0 && !filePageLoading ? (
            !showEmptyDropZone ? (
              <div
                className="file-space-empty file-space-empty--search"
                role="region"
                aria-label={searchLoading ? t("fileSpace.content.searching") : t("fileSpace.content.noResults")}
              >
                <span className="file-space-empty-symbol" aria-hidden="true">
                  {searchLoading ? <LoaderCircle className="is-spinning" size={22} /> : <Search size={22} strokeWidth={1.5} />}
                </span>
                <strong role="status" aria-live="polite">{searchLoading
                  ? t("fileSpace.content.searching")
                  : t("fileSpace.content.noResults")}</strong>
                {!searchLoading ? <span>{t("fileSpace.globalSearch.noResultsDescription")}</span> : null}
                {!searchLoading && (normalizedQuery || hasRestrictiveFilters) ? (
                  <div className="file-space-empty-recovery">
                    {normalizedQuery ? (
                      <button className="is-primary" type="button" onClick={() => setQuery("")}>
                        {t("fileSpace.toolbar.clearSearch")}
                      </button>
                    ) : null}
                    {hasRestrictiveFilters ? (
                      <button type="button" onClick={resetFilters}>{t("fileSpace.toolbar.reset")}</button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : (
              <div
                className={`file-space-empty file-space-empty--drop${isFileDragOver ? " is-drag-over" : ""}`}
                role="region"
                aria-label={t("fileSpace.content.emptyDropTitle")}
                aria-busy={busyAction === "import"}
              >
                <div className="file-space-empty-folder-symbol" aria-hidden="true">
                  <Folder size={34} strokeWidth={1.35} />
                  <span><Upload size={13} strokeWidth={1.8} /></span>
                </div>
                <strong role="status" aria-live="polite" aria-atomic="true">
                  {busyAction === "import"
                    ? t("fileSpace.content.uploading")
                    : isFileDragOver
                      ? t("fileSpace.content.emptyDropActiveTitle")
                      : t("fileSpace.content.emptyTitle")}
                </strong>
                <span>{isFileDragOver
                  ? t("fileSpace.content.emptyDropDescription")
                  : t("fileSpace.content.emptyDescription")}</span>
                {!isFileDragOver ? (
                  <span className="file-space-empty-drop-hint"><Upload size={13} />{t("fileSpace.content.emptyDropTitle")}</span>
                ) : null}
                <div className="file-space-empty-actions">
                  <button className="is-primary" type="button" disabled={!canWrite || Boolean(busyAction)} onClick={() => void importFiles()}>
                    <Upload size={16} />
                    {busyAction === "import" ? t("fileSpace.content.uploading") : t("fileSpace.content.uploadLocalFiles")}
                  </button>
                  <button type="button" disabled={!canWrite || Boolean(busyAction)} onClick={() => openCreateFolder(currentFolder?.id ?? null)}>
                    <FolderPlus size={16} />{t("fileSpace.content.createSubfolder")}
                  </button>
                </div>
              </div>
            )
          ) : null}
        </div>}
        {!isTrashView && !showEmptyDropZone && isFileDragOver ? (
          <div className="file-space-drop-feedback" role="status" aria-live="polite">
            <Upload size={18} />
            <strong>{t("fileSpace.content.dropOverlayTitle")}</strong>
          </div>
        ) : null}
        {error || importFeedback || fileMoveFeedback || trashFeedback || importConflictFeedback || versionNotification ? (
          <div className="file-space-feedback-stack" role="region" aria-label={t("fileSpace.feedback.regionLabel")}>
        {error ? (
          <div className="file-space-import-feedback is-failed" role="alert" aria-live="assertive">
            <span className="file-space-import-feedback-icon"><CircleAlert size={16} /></span>
            <div className="file-space-import-feedback-copy"><strong>{error}</strong></div>
            <button type="button" onClick={() => setError(null)} aria-label={t("fileSpace.feedback.dismiss")}><X size={15} /></button>
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
                  ? t("fileSpace.importFeedback.count", {
                      count: importFeedback.total,
                      processed: importFeedback.processed,
                      total: importFeedback.total,
                    })
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
              <strong>{t(`fileSpace.moveFeedback.${fileMoveFeedback.status}${fileMoveFeedback.fileCount > 1 ? "Many" : ""}Title`, {
                count: fileMoveFeedback.fileCount,
              })}</strong>
              <small>{t(`fileSpace.moveFeedback.${fileMoveFeedback.status}${fileMoveFeedback.fileCount > 1 ? "Many" : ""}Description`, {
                fileName: fileMoveFeedback.fileName,
                count: fileMoveFeedback.fileCount,
                destinationName: fileMoveFeedback.destinationName,
              })}</small>
            </div>
            <button type="button" onClick={() => setFileMoveFeedback(null)} aria-label={t("fileSpace.moveFeedback.dismiss")}><X size={15} /></button>
          </div>
        ) : null}
        {trashFeedback ? (
          <div className="file-space-import-feedback is-completed" role="status" aria-live="polite">
            <span className="file-space-import-feedback-icon"><Check size={16} /></span>
            <div className="file-space-import-feedback-copy"><strong>{trashFeedback}</strong></div>
            <button type="button" onClick={() => setTrashFeedback(null)} aria-label={t("fileSpace.trash.dismiss")}><X size={15} /></button>
          </div>
        ) : null}
        {importConflictFeedback ? (
          <div
            className="file-space-version-notification file-space-identical-import-notification"
            role="status"
            aria-live="polite"
          >
            <span className="file-space-version-notification-icon"><Copy size={17} /></span>
            <div className="file-space-version-notification-copy">
              <strong>{t(importConflictFeedback.files.length === 1
                ? "fileSpace.importConflict.identicalTitleSingle"
                : "fileSpace.importConflict.identicalTitleMany", {
                count: importConflictFeedback.files.length,
              })}</strong>
              <p>{t(importConflictFeedback.files.length === 1
                ? "fileSpace.importConflict.identicalDescriptionSingle"
                : "fileSpace.importConflict.identicalDescriptionMany", {
                count: importConflictFeedback.files.length,
                name: importConflictFeedback.files[0]?.fileName ?? "",
              })}</p>
            </div>
            <div className="file-space-version-notification-actions">
              <button type="button" onClick={() => setImportConflictFeedback(null)}>
                {t("fileSpace.importConflict.acknowledge")}
              </button>
              <button className="is-primary" type="button" onClick={() => void viewIdenticalImportFile()}>
                {t("fileSpace.importConflict.showFile")}
              </button>
            </div>
          </div>
        ) : null}
        {!importConflictFeedback && versionNotification ? (
          <div
            key={versionNotification.versionId}
            className={`file-space-version-notification${importFeedback || fileMoveFeedback || trashFeedback || importConflictFeedback ? " has-transient-feedback" : ""}`}
            role="status"
            aria-live="polite"
          >
            <span className="file-space-version-notification-icon"><Clock3 size={17} /></span>
            <div className="file-space-version-notification-copy">
              <strong>{t(versionNotification.versionId === firstVersionNotificationId
                ? "fileSpace.versionNotification.firstTitle" : "fileSpace.versionNotification.title")}</strong>
              <p>{t(versionNotification.versionId === firstVersionNotificationId
                ? "fileSpace.versionNotification.firstDescription" : "fileSpace.versionNotification.description", {
                name: versionNotification.fileName,
                version: versionNotification.versionNumber,
              })}</p>
            </div>
            <div className="file-space-version-notification-actions">
              <button type="button" onClick={dismissVersionNotification}>
                {t("fileSpace.versionNotification.acknowledge")}
              </button>
              <button className="is-primary" type="button" onClick={() => void viewVersionNotification()}>
                {t("fileSpace.versionNotification.viewChanges")}
              </button>
            </div>
          </div>
        ) : null}
          </div>
        ) : null}
      </main>

      <FileSpaceSearchPanel
        open={isGlobalSearchOpen}
        files={snapshot.files}
        folders={snapshot.folders}
        scopes={searchScopes}
        onOpen={openGlobalSearch}
        onClose={() => setGlobalSearchOpen(false)}
        onOpenFile={openFileFromGlobalSearch}
      />

      <FileSpaceInspector
        hidden={!isInspectorVisible || isTrashView}
        files={selectedFiles}
        artworks={selectedFiles.slice(0, 4).map((file) => <FileArtwork file={file} key={file.id} />)}
        folderNames={[...new Set(selectedFiles.map((file) => (
          file.folderId
            ? (
              snapshot.folders.find((folder) => folder.id === file.folderId)?.relativePath
              ?? file.relativePath.split("/").slice(0, -1).join("/")
            ) || "—"
            : t("fileSpace.inspector.rootFolder")
        )))]}
        timeline={inspectorTimeline}
        timelineLoading={inspectorTimelineLoading}
        formatFileSize={formatFileSize}
        onShowTimeline={() => { if (selectedFile) void openTimeline(selectedFile); }}
        onReveal={() => { if (selectedFile) void runFileCommand("reveal_file_space_file", selectedFile.id, "revealFile"); }}
        onEditTags={() => { if (selectedFile) openTagDialog(selectedFile.id); }}
      />

      {timelinePanelPresence.mounted && timeline && timelineFile ? (
        <div
          className="file-space-timeline-backdrop"
          data-state={timelinePanelPresence.state}
          aria-hidden={!isTimelinePanelOpen}
          inert={!isTimelinePanelOpen}
        >
          <aside ref={timelinePanelRef} className="file-space-timeline-panel" role="dialog" aria-modal="true" aria-label={t("fileSpace.timeline.ariaLabel")} tabIndex={-1}>
            <div className="file-space-timeline-drag-region" data-tauri-drag-region aria-hidden="true" />
            <header>
              <div>
                <span>{t("fileSpace.timeline.taskArtifact")} · {timeline.logicalKey}</span>
                <h2 title={timelineFile.name}>{timelineFile.name}</h2>
              </div>
              <button type="button" onClick={closeTimelinePanel} aria-label={t("fileSpace.timeline.close")}><X size={18} /></button>
            </header>
            <div className="file-space-timeline-body">
              <section className="file-space-version-list" aria-label={t("fileSpace.content.listColumns.versions")} tabIndex={0}>
                {timeline.versions.map((version) => (
                  <div
                    key={version.id}
                    className={`${selectedVersionId === version.id ? "is-selected" : ""}${version.isCurrent ? " is-current" : ""}`}
                  >
                    <button type="button" disabled={timelineBusy} onClick={() => void selectTimelineVersion(version)}>
                      <strong><VersionName number={version.versionNumber} />{version.isCurrent ? ` · ${t("fileSpace.timeline.current")}` : ""}</strong>
                      <span>{version.origin === "task" ? version.taskTitle : t("fileSpace.timeline.userEdit")}</span>
                      {version.roundNumber ? <span>{t("fileSpace.timeline.round", { count: version.roundNumber })} · {version.cellName}</span> : null}
                      <small>{new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(version.producedAt)}</small>
                    </button>
                    <VersionAnnotation workspaceId={timeline.workspaceId} fileId={timeline.fileId} version={version} disabled={timelineBusy} />
                    {!version.isCurrent ? (
                      <button type="button" disabled={timelineBusy} onClick={() => void setCurrentTimelineVersion(version.id)}>{t("fileSpace.timeline.setCurrent")}</button>
                    ) : null}
                  </div>
                ))}
                <InitialVersionHint versions={timeline.versions} />
              </section>
              <section className="file-space-version-preview" aria-label={t("fileSpace.timeline.historicalContent")} tabIndex={0}>
                {timeline.versions.length > 1 ? <FileVersionDiff
                  versions={timeline.versions}
                  beforeVersionId={diffBeforeVersionId}
                  afterVersionId={diffAfterVersionId}
                  status={versionDiffStatus}
                  result={versionDiffResult}
                  error={versionDiffError}
                  onChangeBefore={setDiffBeforeVersionId}
                  onChangeAfter={setDiffAfterVersionId}
                  onSwap={swapComparedVersions}
                  onRetry={() => setVersionDiffRetryToken((token) => token + 1)}
                /> : null}
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
          className={`file-space-file-reorder-overlay${fileLayoutMode === "list" ? " is-list-layout" : ""}`}
          aria-hidden="true"
          style={{
            left: `${internalFileDrag.pointerX - internalFileDrag.offsetX}px`,
            top: `${internalFileDrag.pointerY - internalFileDrag.offsetY}px`,
            width: `${internalFileDrag.cardWidth}px`,
            height: `${internalFileDrag.cardHeight}px`,
            "--file-preview-size": `${fileLayoutMode === "list" ? fileListPreviewSize : previewSize}px`,
          } as React.CSSProperties}
        >
          <FileArtwork file={internallyDraggedFile} onImageDimensions={recordImageDimensions} />
          {internalFileDrag.fileIds.length > 1 ? (
            <span className="file-space-file-drag-count">{internalFileDrag.fileIds.length}</span>
          ) : null}
          <span className="file-space-item-copy">
            <strong>{internallyDraggedFile.name}</strong>
            <small><FileCardMetadata file={internallyDraggedFile} imageDimensions={imageDimensionsByFileVersion[imageDimensionsKey(internallyDraggedFile.id, internallyDraggedFile.updatedAt)]} /></small>
          </span>
        </div>,
        document.body,
      ) : null}

      {contentContextMenu ? (
        <div
          className="file-space-context-menu file-space-content-context-menu"
          ref={contextMenuRef}
          role="menu"
          aria-label={t("fileSpace.contentMenu.title")}
          onKeyDown={handleContextMenuKeyDown}
          style={{ left: contentContextMenu.x, top: contentContextMenu.y }}
        >
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => openCreateTextFile(currentFolder?.id ?? null)}>
            <FilePlus2 size={16} />{t("fileSpace.contentMenu.createFile")}
          </button>
          <span />
          <div className="file-space-content-layout-control" role="group" aria-label={t("fileSpace.contentMenu.layout")}>
            <span><LayoutGrid size={16} /><strong>{t("fileSpace.contentMenu.layout")}</strong></span>
            <div>
              {(["adaptive", "list"] as FileLayoutMode[]).map((mode) => (
                <button
                  className={fileLayoutMode === mode ? "is-selected" : ""}
                  key={mode}
                  type="button"
                  role="menuitemradio"
                  aria-checked={fileLayoutMode === mode}
                  title={t(`fileSpace.contentMenu.layoutOptions.${mode}`)}
                  onClick={() => updateFileLayoutMode(mode)}
                >
                  {t(`fileSpace.contentMenu.layoutOptions.${mode}`)}
                </button>
              ))}
            </div>
          </div>
          <div className="file-space-content-sort-control" role="group" aria-label={t("fileSpace.contentMenu.order")}>
            <span>
              <ArrowUpDown size={16} />
              <strong>{t("fileSpace.contentMenu.sort")}</strong>
            </span>
            <div>
              <button
                className={`${sortOption === "manual" ? "is-selected " : ""}is-automatic-sort`}
                type="button"
                role="menuitemradio"
                aria-checked={sortOption === "manual"}
                title={t("fileSpace.contentMenu.automaticHint")}
                aria-label={t("fileSpace.contentMenu.automaticHint")}
                onClick={() => updateSortOption("manual")}
              >
                {t("fileSpace.contentMenu.automatic")}
              </button>
              <button
                className={sortOption === "updatedAsc" ? "is-selected" : ""}
                type="button"
                role="menuitemradio"
                aria-checked={sortOption === "updatedAsc"}
                title={t("fileSpace.contentMenu.ascendingHint")}
                aria-label={t("fileSpace.contentMenu.ascendingHint")}
                onClick={() => updateSortOption("updatedAsc")}
              >
                <ArrowUpNarrowWide size={15} />
              </button>
              <button
                className={sortOption === "updatedDesc" ? "is-selected" : ""}
                type="button"
                role="menuitemradio"
                aria-checked={sortOption === "updatedDesc"}
                title={t("fileSpace.contentMenu.descendingHint")}
                aria-label={t("fileSpace.contentMenu.descendingHint")}
                onClick={() => updateSortOption("updatedDesc")}
              >
                <ArrowDownWideNarrow size={15} />
              </button>
            </div>
          </div>
          <span />
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={toggleSidebarVisibility}>
            <FolderOpen size={16} />{t(isSidebarVisible ? "fileSpace.contentMenu.hideFolders" : "fileSpace.contentMenu.showFolders")}
          </button>
          <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={toggleInspectorVisibility}>
            <Info size={16} />{t(isInspectorVisible ? "fileSpace.contentMenu.hideInfo" : "fileSpace.contentMenu.showInfo")}
          </button>
        </div>
      ) : null}

      {folderContextMenu ? (
        <div
          className="file-space-context-menu"
          ref={contextMenuRef}
          role="menu"
          onKeyDown={handleContextMenuKeyDown}
          style={{ left: folderContextMenu.x, top: folderContextMenu.y }}
        >
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => openCreateFolder(folderContextMenu.folderId)}>
            <FolderPlus size={16} />{t("fileSpace.folderMenu.newSubfolder")}
          </button>
          {folderContextMenuExpandableIds.length > 0 ? (
            <button type="button" role="menuitem" disabled={Boolean(busyAction)} onClick={() => toggleFolderSubtree(folderContextMenu.folderId)}>
              {isFolderContextSubtreeFullyExpanded ? <ChevronsDownUp size={16} /> : <ChevronsUpDown size={16} />}
              {t(isFolderContextSubtreeFullyExpanded ? "fileSpace.folderMenu.collapseAll" : "fileSpace.folderMenu.expandAll")}
            </button>
          ) : null}
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => openRenameFolder(folderContextMenu.folderId)}>
            <Pencil size={15} />{t("fileSpace.folderMenu.rename")}
          </button>
          <span />
          <button className="is-danger" type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => requestDeleteFolder(folderContextMenu.folderId)}>
            <Trash2 size={16} />{t("fileSpace.folderMenu.delete")}
          </button>
        </div>
      ) : null}

      {fileContextMenu ? (
        <div
          className="file-space-context-menu file-space-file-context-menu"
          ref={contextMenuRef}
          role="menu"
          onKeyDown={handleContextMenuKeyDown}
          style={{ left: fileContextMenu.x, top: fileContextMenu.y }}
        >
          {(snapshot.files.find((file) => file.id === fileContextMenu.fileId)?.versionCount ?? 0) > 0 || externalWorkspace?.source ? (
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
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => void runFileCommand("reveal_file_space_file", fileContextMenu.fileId, "revealFile")}>
            <FolderOpen size={16} />{t("fileSpace.fileMenu.reveal")}
          </button>
          <span />
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => openRenameFile(fileContextMenu.fileId)}>
            <Pencil size={15} />{t("fileSpace.fileMenu.rename")}
          </button>
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => openTagDialog(fileContextMenu.fileId)}>
            <Tag size={15} />{t("fileSpace.fileMenu.tags")}
          </button>
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => void runFileCommand("copy_file_space_file", fileContextMenu.fileId, "copyFile")}>
            <Copy size={15} />{t("fileSpace.fileMenu.copyFile")}
          </button>
          <button type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => void runFileCommand("copy_file_space_file_path", fileContextMenu.fileId, "copyPath")}>
            <FileText size={15} />{t("fileSpace.fileMenu.copyPath")}
          </button>
          <span />
          <button className="is-danger" type="button" role="menuitem" disabled={!canWrite || Boolean(busyAction)} onClick={() => requestDeleteFile(fileContextMenu.fileId)}>
            <Trash2 size={16} />{t("fileSpace.fileMenu.delete")}
          </button>
        </div>
      ) : null}

      {createDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${createDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <form className={`file-space-dialog${createDialogPresence.state === "open" ? " is-open" : ""}`} role="dialog" aria-modal="true" aria-label={t("fileSpace.createFolder.title")} tabIndex={-1} onSubmit={(event) => { event.preventDefault(); void createFolder(); }}>
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

      {createFileDialogPresence.mounted && createFileFormat ? (
        <div
          className={`file-space-dialog-backdrop${createFileDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <form
            className={`file-space-dialog${createFileDialogPresence.state === "open" ? " is-open" : ""}`}
            role="dialog"
            aria-modal="true"
            aria-label={t("fileSpace.createFile.dialogTitle")}
            tabIndex={-1}
            onSubmit={(event) => { event.preventDefault(); void createTextFile(); }}
          >
            <header>
              <div>
                <h2>{t("fileSpace.createFile.dialogTitle")}</h2>
                <p>{t("fileSpace.createFile.description")}</p>
              </div>
              <button type="button" disabled={busyAction === "create"} onClick={() => setCreateFileFormat(null)} aria-label={t("fileSpace.createFile.cancel")}><X size={17} /></button>
            </header>
            <fieldset className="file-space-create-file-format">
              <legend>{t("fileSpace.createFile.formatLabel")}</legend>
              <div role="radiogroup" aria-label={t("fileSpace.createFile.formatLabel")}>
                {(["md", "txt"] as const).map((format) => (
                  <button
                    key={format}
                    type="button"
                    role="radio"
                    aria-checked={createFileFormat === format}
                    className={createFileFormat === format ? "is-selected" : ""}
                    disabled={busyAction === "create"}
                    onClick={() => selectCreateTextFileFormat(format)}
                  >
                    {t(`fileSpace.createFile.formats.${format}`)}
                  </button>
                ))}
              </div>
            </fieldset>
            <label>
              <span>{t("fileSpace.createFile.label")}</span>
              <span className="file-space-create-file-name-field">
                <input
                  ref={createFileNameRef}
                  value={createFileName}
                  aria-describedby="file-space-create-file-extension"
                  disabled={busyAction === "create"}
                  onChange={(event) => setCreateFileName(event.target.value)}
                  spellCheck={false}
                />
                <span id="file-space-create-file-extension" aria-label={t("fileSpace.createFile.fixedExtension", { extension: createFileFormat })}>
                  .{createFileFormat}
                </span>
              </span>
            </label>
            <footer>
              <button type="button" disabled={busyAction === "create"} onClick={() => setCreateFileFormat(null)}>{t("fileSpace.createFile.cancel")}</button>
              <button className="is-primary" type="submit" disabled={!createFileName.trim() || Boolean(busyAction)}>
                {busyAction === "create" ? t("fileSpace.createFile.submitting") : t("fileSpace.createFile.submit")}
              </button>
            </footer>
          </form>
        </div>
      ) : null}

      {renameDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${renameDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <form className={`file-space-dialog${renameDialogPresence.state === "open" ? " is-open" : ""}`} role="dialog" aria-modal="true" aria-label={t("fileSpace.renameFolder.title")} tabIndex={-1} onSubmit={(event) => { event.preventDefault(); void renameFolder(); }}>
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
        >
          <form className={`file-space-dialog${renameFileDialogPresence.state === "open" ? " is-open" : ""}`} role="dialog" aria-modal="true" aria-label={t("fileSpace.renameFile.title")} tabIndex={-1} onSubmit={(event) => { event.preventDefault(); void renameFile(); }}>
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
        >
          <form className={`file-space-dialog${tagDialogPresence.state === "open" ? " is-open" : ""}`} role="dialog" aria-modal="true" aria-label={t("fileSpace.tags.title")} tabIndex={-1} onSubmit={(event) => { event.preventDefault(); void saveFileTags(); }}>
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

      {importConflictDialogPresence.mounted && pendingFileImportConflict ? (
        <div
          className={`file-space-dialog-backdrop${importConflictDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <div
            className={`file-space-dialog file-space-import-conflict-dialog${importConflictDialogPresence.state === "open" ? " is-open" : ""}`}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="file-space-import-conflict-title"
            aria-describedby="file-space-import-conflict-description"
            tabIndex={-1}
          >
            <header>
              <div>
                <h2 id="file-space-import-conflict-title">
                  {t("fileSpace.importConflict.title", {
                    count: pendingFileImportConflict.conflicts.length,
                    name: pendingFileImportConflict.conflicts[0]?.fileName ?? "",
                  })}
                </h2>
                <p id="file-space-import-conflict-description">
                  {t("fileSpace.importConflict.description", {
                    count: pendingFileImportConflict.conflicts.length,
                  })}
                </p>
              </div>
              <button
                type="button"
                disabled={busyAction === "import"}
                onClick={() => setPendingFileImportConflict(null)}
                aria-label={t("fileSpace.importConflict.cancel")}
              >
                <X size={17} />
              </button>
            </header>
            <div className="file-space-import-conflict-list" role="list">
              {pendingFileImportConflict.conflicts.map((conflict) => (
                <div key={`${conflict.existingFileId}:${conflict.relativePath}`} role="listitem">
                  <span className="file-space-import-conflict-icon"><Clock3 size={16} /></span>
                  <span>
                    <strong>{conflict.fileName}</strong>
                    <small>{t("fileSpace.importConflict.fileDetails", {
                      existingSize: formatFileSize(conflict.existingSizeBytes),
                      incomingSize: formatFileSize(conflict.incomingSizeBytes),
                      version: conflict.existingVersion,
                    })}</small>
                  </span>
                </div>
              ))}
            </div>
            <footer>
              <button
                ref={importConflictCancelRef}
                type="button"
                disabled={busyAction === "import"}
                onClick={() => setPendingFileImportConflict(null)}
              >
                {t("fileSpace.importConflict.cancel")}
              </button>
              <button
                type="button"
                disabled={Boolean(busyAction)}
                onClick={() => void resolveFileImportConflict("rename")}
              >
                {t("fileSpace.importConflict.rename")}
              </button>
              <button
                className="is-primary"
                type="button"
                disabled={Boolean(busyAction)}
                onClick={() => void resolveFileImportConflict("latestVersion")}
              >
                {busyAction === "import"
                  ? t("fileSpace.importConflict.applying")
                  : t("fileSpace.importConflict.latestVersion")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {deleteDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${deleteDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <div className={`file-space-dialog file-space-delete-dialog${deleteDialogPresence.state === "open" ? " is-open" : ""}`} role="alertdialog" aria-modal="true" aria-label={t("fileSpace.deleteFolder.title")} tabIndex={-1}>
            <header>
              <div><h2>{t("fileSpace.deleteFolder.title")}</h2><p>{t("fileSpace.deleteFolder.description", { name: snapshot.folders.find((folder) => folder.id === deleteFolderId)?.name ?? "" })}</p></div>
              <button type="button" onClick={() => setDeleteFolderId(null)} aria-label={t("fileSpace.deleteFolder.cancel")}><X size={17} /></button>
            </header>
            <footer>
              <button ref={deleteFolderCancelRef} type="button" onClick={() => setDeleteFolderId(null)}>{t("fileSpace.deleteFolder.cancel")}</button>
              <button className="is-primary" type="button" disabled={Boolean(busyAction)} onClick={() => void deleteFolder()}>
                {busyAction === "delete" ? t("fileSpace.deleteFolder.submitting") : t("fileSpace.deleteFolder.submit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {restoreTrashDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${restoreTrashDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <div
            className={`file-space-dialog file-space-confirm-dialog${restoreTrashDialogPresence.state === "open" ? " is-open" : ""}`}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="file-space-restore-trash-title"
            aria-describedby="file-space-restore-trash-description"
            tabIndex={-1}
          >
            <header>
              <div>
                <h2 id="file-space-restore-trash-title">
                  {t("fileSpace.trash.confirmTitle", { name: trashItemPendingRestore?.name ?? "" })}
                </h2>
                <p id="file-space-restore-trash-description">
                  {t("fileSpace.trash.confirmDescription")}
                </p>
              </div>
              <button
                type="button"
                disabled={busyAction === "restore"}
                onClick={() => setRestoreConfirmationEntryId(null)}
                aria-label={t("fileSpace.trash.confirmCancel")}
              >
                <X size={17} />
              </button>
            </header>
            <footer>
              <button
                ref={restoreTrashCancelRef}
                type="button"
                disabled={busyAction === "restore"}
                onClick={() => setRestoreConfirmationEntryId(null)}
              >
                {t("fileSpace.trash.confirmCancel")}
              </button>
              <button
                className="is-primary"
                type="button"
                disabled={Boolean(busyAction)}
                onClick={() => void restoreTrashEntry()}
              >
                {busyAction === "restore"
                  ? t("fileSpace.trash.restoring")
                  : t("fileSpace.trash.confirmSubmit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {emptyTrashDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${emptyTrashDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <div
            className={`file-space-dialog file-space-confirm-dialog file-space-empty-trash-dialog${emptyTrashDialogPresence.state === "open" ? " is-open" : ""}`}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="file-space-empty-trash-title"
            aria-describedby="file-space-empty-trash-description"
            tabIndex={-1}
          >
            <header>
              <div>
                <h2 id="file-space-empty-trash-title">{t("fileSpace.trash.emptyConfirmTitle")}</h2>
                <p id="file-space-empty-trash-description">
                  {t("fileSpace.trash.emptyConfirmDescription", {
                    count: emptyTrashEntryIds.length,
                  })}
                </p>
              </div>
              <button
                type="button"
                disabled={busyAction === "purgeTrash"}
                onClick={() => setEmptyTrashConfirmationOpen(false)}
                aria-label={t("fileSpace.trash.emptyConfirmCancel")}
              >
                <X size={17} />
              </button>
            </header>
            <footer>
              <button
                ref={emptyTrashCancelRef}
                type="button"
                disabled={busyAction === "purgeTrash"}
                onClick={() => setEmptyTrashConfirmationOpen(false)}
              >
                {t("fileSpace.trash.emptyConfirmCancel")}
              </button>
              <button
                className="is-danger"
                type="button"
                disabled={Boolean(busyAction)}
                onClick={() => void emptyTrash()}
              >
                {busyAction === "purgeTrash"
                  ? t("fileSpace.trash.cleaning")
                  : t("fileSpace.trash.emptyConfirmSubmit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {deleteFileDialogPresence.mounted ? (
        <div
          className={`file-space-dialog-backdrop${deleteFileDialogPresence.state === "open" ? " is-open" : ""}`}
        >
          <div className={`file-space-dialog file-space-delete-dialog${deleteFileDialogPresence.state === "open" ? " is-open" : ""}`} role="alertdialog" aria-modal="true" aria-label={t("fileSpace.deleteFile.title")} tabIndex={-1}>
            <header>
              <div>
                <h2>{t("fileSpace.deleteFile.title")}</h2>
                <p>{t("fileSpace.deleteFile.description", { name: deletedFile?.name ?? "" })}</p>
              </div>
              <button type="button" onClick={() => setDeleteFileId(null)} aria-label={t("fileSpace.deleteFile.cancel")}><X size={17} /></button>
            </header>
            <footer>
              <button ref={deleteFileCancelRef} type="button" onClick={() => setDeleteFileId(null)}>{t("fileSpace.deleteFile.cancel")}</button>
              <button className="is-primary" type="button" disabled={Boolean(busyAction)} onClick={() => void deleteFile()}>
                {busyAction === "delete" ? t("fileSpace.deleteFile.submitting") : t("fileSpace.deleteFile.submit")}
              </button>
            </footer>
          </div>
        </div>
      ) : null}
    </section>
  );
}
