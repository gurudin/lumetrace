import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  ArchiveRestore,
  CircleAlert,
  CircleCheck,
  Download,
  FolderCog,
  HardDriveDownload,
  Info,
  LoaderCircle,
  MessageSquare,
  RotateCcw,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import appPackage from "../../../package.json";
import lumeTraceLogo from "../../../src-tauri/icons/icon.png";
import { usePresence } from "../../shared/ui/usePresence";
import { FileSpacePreferences } from "./FileSpacePreferences";
import { FileSpaceFeedback } from "./FileSpaceFeedback";
import {
  FileSpaceWorkspaceSettings,
  type FileSpaceWorkspaceDirectory,
  type FileSpaceWorkspaceMutation,
} from "./FileSpaceWorkspaceMenu";
import {
  semanticStatusView,
  shouldPollSemanticStatus,
  type SemanticStatusReadState,
} from "./semanticStatusPresentation";
import {
  openAiServiceSettingsEventName,
  openBackgroundStatusEventName,
  preferencesSectionForOpenEvent,
  type PreferencesSection,
} from "./preferencesNavigation";

type SettingsPanel = "workspace" | "about" | "preferences" | "semantic" | "backup" | "restore" | "feedback" | "privacy";
type BackupStatus = "idle" | "exporting" | "success" | "error";
type RestoreStatus = "idle" | "ready" | "restoring" | "success" | "error";

interface BackupFeedback {
  status: BackupStatus;
  path?: string;
  error?: string;
}

interface BackupResult {
  path: string;
}

interface BackupInspection {
  path: string;
  exportedAt: number;
  workspaceName: string;
  workspaceFileCount: number;
  workspaceSizeBytes: number;
}

interface RestoreFeedback {
  status: RestoreStatus;
  inspection?: BackupInspection;
  destinationDirectory?: string;
  restoredRootPath?: string;
  error?: string;
}

interface RestoreResult {
  rootPath: string | null;
}

interface SemanticSearchStatus {
  state: "notInstalled" | "downloading" | "validating" | "indexing" | "ready" | "failed";
  installed: boolean;
  modelId: string;
  modelName: string;
  dimensions: number;
  downloadedBytes: number;
  totalDownloadBytes: number;
  currentFile?: string | null;
  indexedFiles: number;
  totalFiles: number;
  pendingFiles: number;
  failedFiles: number;
  error?: string | null;
}

const menuItems: readonly SettingsPanel[] = [
  "workspace",
  "preferences",
  "semantic",
  "backup",
  "restore",
  "feedback",
  "privacy",
  "about",
];

const privacyUpdatedAt = new Date(2026, 8, 4);

interface FileSpaceSettingsMenuProps<TSnapshot> {
  workspaceDirectory: FileSpaceWorkspaceDirectory | null;
  workspaceDisabled?: boolean;
  onWorkspaceChanged: (mutation: FileSpaceWorkspaceMutation<TSnapshot>) => void;
  onWorkspaceDirectoryChanged: (directory: FileSpaceWorkspaceDirectory) => void;
}

const semanticPreviewStatus: SemanticSearchStatus = {
  state: "notInstalled",
  installed: false,
  modelId: "multilingual-e5-small-int8-v1",
  modelName: "Multilingual E5 Small",
  dimensions: 384,
  downloadedBytes: 0,
  totalDownloadBytes: 135_392_183,
  indexedFiles: 0,
  totalFiles: 0,
  pendingFiles: 0,
  failedFiles: 0,
};

function backupFileName(now = new Date()) {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `Lume Trace ${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}-${pad(now.getMinutes())}-${pad(now.getSeconds())}.lumetrace`;
}

function formatBackupSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024;
    unit = units[index];
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}

export function FileSpaceSettingsMenu<TSnapshot>({
  workspaceDirectory,
  workspaceDisabled = false,
  onWorkspaceChanged,
  onWorkspaceDirectoryChanged,
}: FileSpaceSettingsMenuProps<TSnapshot>) {
  const { t, i18n } = useTranslation();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const dialogCloseRef = useRef<HTMLButtonElement>(null);
  const semanticStatusRequestIdRef = useRef(0);
  const semanticStatusPendingRequestRef = useRef<number | null>(null);
  const semanticActionBusyRef = useRef(false);
  const [isMenuOpen, setMenuOpen] = useState(false);
  const [activePanel, setActivePanel] = useState<SettingsPanel | null>(null);
  const [preferencesInitialSection, setPreferencesInitialSection] = useState<PreferencesSection>("appearance");
  const [preferencesNavigationRequest, setPreferencesNavigationRequest] = useState(0);
  const [backupFeedback, setBackupFeedback] = useState<BackupFeedback>({ status: "idle" });
  const [restoreFeedback, setRestoreFeedback] = useState<RestoreFeedback>({ status: "idle" });
  const [semanticStatus, setSemanticStatus] = useState<SemanticSearchStatus>(semanticPreviewStatus);
  const [semanticStatusReadState, setSemanticStatusReadState] = useState<SemanticStatusReadState>("idle");
  const [semanticStatusReadError, setSemanticStatusReadError] = useState<string | null>(null);
  const [semanticBusy, setSemanticBusy] = useState(false);
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [confirmingModelRemoval, setConfirmingModelRemoval] = useState(false);
  const [appVersion, setAppVersion] = useState(appPackage.version);
  const menuPresence = usePresence(isMenuOpen, 120);
  const dialogPresence = usePresence(Boolean(activePanel), 160);

  const closeMenu = useCallback((restoreFocus = false) => {
    setMenuOpen(false);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  }, []);

  const closeDialog = useCallback(() => {
    if (restoreFeedback.status === "restoring" || workspaceBusy) return;
    setActivePanel(null);
    window.setTimeout(() => triggerRef.current?.focus(), 170);
  }, [restoreFeedback.status, workspaceBusy]);

  const openPanel = useCallback((panel: SettingsPanel) => {
    setMenuOpen(false);
    if (panel === "semantic") {
      setConfirmingModelRemoval(false);
      setSemanticStatusReadState("loading");
      setSemanticStatusReadError(null);
    }
    setActivePanel(panel);
  }, []);

  const openPreferences = useCallback((section: PreferencesSection = "appearance") => {
    setMenuOpen(false);
    setPreferencesInitialSection(section);
    setPreferencesNavigationRequest((request) => request + 1);
    setActivePanel("preferences");
  }, []);

  const refreshSemanticStatus = useCallback(async (showLoading = false) => {
    if (semanticActionBusyRef.current) return;
    if (!showLoading && semanticStatusPendingRequestRef.current !== null) return;
    const requestId = semanticStatusRequestIdRef.current + 1;
    semanticStatusRequestIdRef.current = requestId;
    semanticStatusPendingRequestRef.current = requestId;
    if (showLoading) {
      setSemanticStatusReadState("loading");
      setSemanticStatusReadError(null);
    }
    try {
      if (!isTauri()) {
        const previewState = new URLSearchParams(window.location.search).get("semanticState");
        if (previewState === "unavailable") {
          throw new Error(t("fileSpace.settings.semantic.previewUnavailable"));
        }
        const state = previewState === "downloading"
          || previewState === "validating"
          || previewState === "indexing"
          || previewState === "ready"
          || previewState === "failed"
          ? previewState
          : "notInstalled";
        setSemanticStatus({
          ...semanticPreviewStatus,
          state,
          installed: state === "indexing" || state === "ready" || (state === "failed" && previewState === "failed"),
          downloadedBytes: state === "downloading" ? 62_400_000 : state === "notInstalled" ? 0 : semanticPreviewStatus.totalDownloadBytes,
          indexedFiles: state === "indexing" ? 328 : state === "ready" ? 1_200 : 0,
          totalFiles: state === "indexing" || state === "ready" ? 1_200 : 0,
          pendingFiles: state === "indexing" ? 872 : 0,
          failedFiles: state === "failed" ? 1 : 0,
          error: state === "failed" ? t("fileSpace.settings.semantic.previewFailure") : null,
        });
      } else {
        const status = await invoke<SemanticSearchStatus>("get_semantic_search_status");
        if (semanticStatusRequestIdRef.current !== requestId) return;
        setSemanticStatus(status);
      }
      if (semanticStatusRequestIdRef.current !== requestId) return;
      setSemanticStatusReadState("ready");
      setSemanticStatusReadError(null);
    } catch (error) {
      if (semanticStatusRequestIdRef.current !== requestId) return;
      setSemanticStatusReadState("error");
      setSemanticStatusReadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (semanticStatusPendingRequestRef.current === requestId) {
        semanticStatusPendingRequestRef.current = null;
      }
    }
  }, [t]);

  const runSemanticAction = async (
    command: "install_semantic_search_model"
      | "cancel_semantic_search_model_download"
      | "retry_semantic_search_index"
      | "remove_semantic_search_model",
  ) => {
    if (semanticActionBusyRef.current) return;
    semanticActionBusyRef.current = true;
    semanticStatusRequestIdRef.current += 1;
    semanticStatusPendingRequestRef.current = null;
    setSemanticBusy(true);
    try {
      if (isTauri()) {
        const status = await invoke<SemanticSearchStatus>(command);
        setSemanticStatus(status);
      }
      setSemanticStatusReadState("ready");
      setSemanticStatusReadError(null);
      if (command === "remove_semantic_search_model") setConfirmingModelRemoval(false);
    } catch (error) {
      setSemanticStatus((current) => ({
        ...current,
        state: "failed",
        error: error instanceof Error ? error.message : String(error),
      }));
    } finally {
      semanticActionBusyRef.current = false;
      setSemanticBusy(false);
    }
  };

  const exportBackup = async () => {
    if (backupFeedback.status === "exporting") return;
    setMenuOpen(false);
    try {
      const destinationPath = await save({
        title: t("fileSpace.settings.backupTitle"),
        defaultPath: backupFileName(),
        filters: [{
          name: t("fileSpace.settings.backupFileType"),
          extensions: ["lumetrace"],
        }],
      });
      if (!destinationPath) {
        window.requestAnimationFrame(() => triggerRef.current?.focus());
        return;
      }
      setBackupFeedback({ status: "exporting" });
      setActivePanel("backup");
      const result = await invoke<BackupResult>("export_file_space_backup", { destinationPath });
      setBackupFeedback({ status: "success", path: result.path });
    } catch (error) {
      setBackupFeedback({
        status: "error",
        error: error instanceof Error ? error.message : String(error),
      });
      setActivePanel("backup");
    }
  };

  const chooseRestoreBackup = async () => {
    if (restoreFeedback.status === "restoring") return;
    setMenuOpen(false);
    try {
      const backupPath = await open({
        title: t("fileSpace.settings.restoreChooseBackup"),
        multiple: false,
        directory: false,
        filters: [{
          name: t("fileSpace.settings.backupFileType"),
          extensions: ["lumetrace", "lumetrace-backup"],
        }],
      });
      if (typeof backupPath !== "string") {
        window.requestAnimationFrame(() => triggerRef.current?.focus());
        return;
      }
      const inspection = await invoke<BackupInspection>("inspect_file_space_backup", {
        path: backupPath,
      });
      const destinationDirectory = await open({
        title: t("fileSpace.settings.restoreChooseDestination"),
        multiple: false,
        directory: true,
      });
      if (typeof destinationDirectory !== "string") {
        window.requestAnimationFrame(() => triggerRef.current?.focus());
        return;
      }
      setRestoreFeedback({ status: "ready", inspection, destinationDirectory });
      setActivePanel("restore");
    } catch (error) {
      setRestoreFeedback({
        status: "error",
        error: error instanceof Error ? error.message : String(error),
      });
      setActivePanel("restore");
    }
  };

  const confirmRestoreBackup = async () => {
    const { inspection, destinationDirectory } = restoreFeedback;
    if (!inspection || !destinationDirectory || restoreFeedback.status !== "ready") return;
    setRestoreFeedback({ ...restoreFeedback, status: "restoring", error: undefined });
    try {
      const result = await invoke<RestoreResult>("restore_file_space_backup", {
        backupPath: inspection.path,
        destinationDirectory,
      });
      window.dispatchEvent(new CustomEvent("lumetrace:file-space-snapshot", { detail: result }));
      setRestoreFeedback({
        status: "success",
        inspection,
        destinationDirectory,
        restoredRootPath: result.rootPath ?? destinationDirectory,
      });
    } catch (error) {
      setRestoreFeedback({
        status: "error",
        inspection,
        destinationDirectory,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  useEffect(() => {
    if (activePanel !== "about" || !isTauri()) return undefined;
    let cancelled = false;
    void getVersion()
      .then((version) => {
        if (!cancelled) setAppVersion(version);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [activePanel]);

  useEffect(() => {
    if (!isMenuOpen) return undefined;
    const frame = window.requestAnimationFrame(() => {
      menuRef.current?.focus();
    });
    const handlePointerDown = (event: globalThis.PointerEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      closeMenu(false);
    };
    window.addEventListener("pointerdown", handlePointerDown, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("pointerdown", handlePointerDown, true);
    };
  }, [closeMenu, isMenuOpen]);

  useEffect(() => {
    if (!activePanel || !dialogPresence.mounted) return undefined;
    const frame = window.requestAnimationFrame(() => dialogCloseRef.current?.focus());
    const handleKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        if (event.target instanceof Element && event.target.closest(".mac-select-menu")) return;
        event.preventDefault();
        event.stopPropagation();
        closeDialog();
        return;
      }
      if (event.key !== "Tab" || !dialogRef.current) return;
      const focusable = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])',
      )).filter((element) => element.getClientRects().length > 0);
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (!dialogRef.current.contains(document.activeElement)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [activePanel, closeDialog, dialogPresence.mounted]);

  useEffect(() => {
    if (activePanel !== "semantic") {
      semanticStatusRequestIdRef.current += 1;
      semanticStatusPendingRequestRef.current = null;
      return undefined;
    }
    if (semanticBusy) return undefined;
    void refreshSemanticStatus(true);
    return undefined;
  }, [activePanel, refreshSemanticStatus, semanticBusy]);

  useEffect(() => {
    if (
      activePanel !== "semantic"
      || !shouldPollSemanticStatus(semanticStatusReadState, semanticStatus.state, semanticBusy)
    ) return undefined;
    const interval = window.setInterval(() => {
      void refreshSemanticStatus();
    }, 900);
    return () => window.clearInterval(interval);
  }, [activePanel, refreshSemanticStatus, semanticBusy, semanticStatus.state, semanticStatusReadState]);

  useEffect(() => {
    const openPreferencesSection = (event: Event) => {
      const section = preferencesSectionForOpenEvent(event.type);
      if (section) openPreferences(section);
    };
    window.addEventListener(openAiServiceSettingsEventName, openPreferencesSection);
    window.addEventListener(openBackgroundStatusEventName, openPreferencesSection);
    return () => {
      window.removeEventListener(openAiServiceSettingsEventName, openPreferencesSection);
      window.removeEventListener(openBackgroundStatusEventName, openPreferencesSection);
    };
  }, [openPreferences]);

  const handleMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? []);
    const currentIndex = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
      return;
    }
    if (event.key === "Tab") {
      closeMenu(false);
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const nextIndex = event.key === "Home" || (event.key === "ArrowDown" && currentIndex < 0)
      ? 0
      : event.key === "End" || (event.key === "ArrowUp" && currentIndex < 0)
        ? items.length - 1
        : (currentIndex + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[nextIndex]?.focus();
  };

  const handleBackdropPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget) closeDialog();
  };

  const panelTitle = activePanel === "backup" || activePanel === "restore" || activePanel === "semantic"
    ? t(`fileSpace.settings.${activePanel}Title`)
    : activePanel
      ? t(`fileSpace.settings.${activePanel}`)
      : "";
  const panelClassName = activePanel;
  const semanticView = semanticStatusView(semanticStatusReadState);

  return (
    <div className="file-space-settings-menu">
      <button
        ref={triggerRef}
        className="file-space-settings-trigger"
        type="button"
        aria-label={t("fileSpace.settings.open")}
        title={t("fileSpace.settings.open")}
        aria-haspopup="menu"
        aria-expanded={isMenuOpen}
        onClick={() => setMenuOpen((open) => !open)}
      >
        <Settings size={17} />
      </button>

      {menuPresence.mounted ? (
        <div
          ref={menuRef}
          className="file-space-settings-popover"
          data-state={menuPresence.state}
          role="menu"
          tabIndex={-1}
          aria-label={t("fileSpace.settings.menuLabel")}
          onKeyDown={handleMenuKeyDown}
        >
          {menuItems.map((item) => {
            const Icon = item === "workspace"
              ? FolderCog
              : item === "about"
              ? Info
              : item === "preferences"
                ? SlidersHorizontal
                : item === "semantic"
                  ? Sparkles
                : item === "backup"
                  ? Download
                  : item === "restore"
                    ? ArchiveRestore
                  : item === "feedback"
                    ? MessageSquare
                  : ShieldCheck;
            return (
              <button
                key={item}
                className={item === "semantic" || item === "backup" || item === "feedback"
                  ? "is-separated"
                  : undefined}
                type="button"
                role="menuitem"
                disabled={
                  (item === "backup" && backupFeedback.status === "exporting")
                  || (item === "restore" && restoreFeedback.status === "restoring")
                }
                onClick={() => {
                  if (item === "backup") void exportBackup();
                  else if (item === "restore") void chooseRestoreBackup();
                  else if (item === "preferences") openPreferences();
                  else openPanel(item);
                }}
              >
                <Icon size={17} />
                <span>{item === "semantic"
                  ? t("fileSpace.settings.semantic.menu")
                  : t(`fileSpace.settings.${item}`)}</span>
              </button>
            );
          })}
        </div>
      ) : null}

      {dialogPresence.mounted ? createPortal(
        <div
          className={`file-space-dialog-backdrop file-space-settings-backdrop${dialogPresence.state === "open" ? " is-open" : ""}`}
          onPointerDown={handleBackdropPointerDown}
        >
          <section
            ref={dialogRef}
            className={`file-space-dialog file-space-settings-dialog${panelClassName ? ` is-${panelClassName}` : ""}${dialogPresence.state === "open" ? " is-open" : ""}`}
            role="dialog"
            aria-modal="true"
            aria-labelledby="file-space-settings-dialog-title"
            tabIndex={-1}
          >
            <header>
              <div><h2 id="file-space-settings-dialog-title">{panelTitle}</h2></div>
              <button
                ref={dialogCloseRef}
                type="button"
                disabled={restoreFeedback.status === "restoring" || workspaceBusy}
                onClick={closeDialog}
                aria-label={t("fileSpace.settings.close")}
              >
                <X size={17} />
              </button>
            </header>

            {activePanel === "workspace" ? (
              <FileSpaceWorkspaceSettings<TSnapshot>
                directory={workspaceDirectory}
                disabled={workspaceDisabled}
                onWorkspaceChanged={onWorkspaceChanged}
                onDirectoryChanged={onWorkspaceDirectoryChanged}
                onBusyChange={setWorkspaceBusy}
                onDone={closeDialog}
              />
            ) : null}

            {activePanel === "about" ? (
              <div className="file-space-settings-about">
                <img src={lumeTraceLogo} alt="" />
                <div className="file-space-settings-about-copy">
                  <strong>Lume Trace</strong>
                  <span className="file-space-settings-about-version">
                    {t("fileSpace.settings.aboutVersion", { version: appVersion })}
                  </span>
                  <p>{t("fileSpace.settings.aboutDescription")}</p>
                </div>
                <button
                  className="file-space-settings-about-privacy"
                  type="button"
                  onClick={() => openPanel("privacy")}
                >
                  <ShieldCheck size={15} />
                  {t("fileSpace.settings.privacy")}
                </button>
                <small>{t("fileSpace.settings.aboutCopyright", { year: new Date().getFullYear() })}</small>
              </div>
            ) : null}

            {activePanel === "preferences" ? (
              <FileSpacePreferences
                initialSection={preferencesInitialSection}
                navigationRequest={preferencesNavigationRequest}
                onClose={closeDialog}
                onOpenSemantic={() => openPanel("semantic")}
              />
            ) : null}

            {activePanel === "semantic" ? (
              <div className="file-space-settings-semantic" aria-live="polite">
                <p className="file-space-settings-semantic-intro">
                  {t("fileSpace.settings.semantic.description")}
                </p>
                <section className="file-space-settings-semantic-model" aria-label={t("fileSpace.settings.semantic.modelLabel")}>
                  <div className="file-space-settings-semantic-model-heading">
                    <span className="file-space-settings-semantic-model-icon" aria-hidden="true">
                      E5
                    </span>
                    <div>
                      <strong>{semanticStatus.modelName}</strong>
                      <span>{t("fileSpace.settings.semantic.modelMeta", {
                        size: formatBackupSize(semanticStatus.totalDownloadBytes),
                        dimensions: semanticStatus.dimensions,
                      })}</span>
                    </div>
                  </div>

                  {semanticView === "loading" ? (
                    <div className="file-space-settings-semantic-state file-space-settings-state is-loading">
                      <LoaderCircle className="is-spinning" size={17} />
                      <div>
                        <strong>{t("fileSpace.settings.semantic.statusLoadingTitle")}</strong>
                        <span>{t("fileSpace.settings.semantic.statusLoadingDescription")}</span>
                      </div>
                    </div>
                  ) : semanticView === "error" ? (
                    <>
                      <div className="file-space-settings-semantic-state file-space-settings-state is-read-error" role="alert">
                        <CircleAlert size={17} />
                        <div>
                          <strong>{t("fileSpace.settings.semantic.statusReadFailedTitle")}</strong>
                          <span>{t("fileSpace.settings.semantic.statusReadFailedDescription")}</span>
                        </div>
                      </div>
                      {semanticStatusReadError ? (
                        <small className="file-space-settings-semantic-error">{semanticStatusReadError}</small>
                      ) : null}
                      <div className="file-space-settings-semantic-model-actions">
                        <button type="button" onClick={() => void refreshSemanticStatus(true)}>
                          <RotateCcw size={14} />
                          {t("fileSpace.settings.semantic.retryStatus")}
                        </button>
                      </div>
                    </>
                  ) : (
                    <>
                      <div className={`file-space-settings-semantic-state file-space-settings-state is-${semanticStatus.state}`}>
                        {semanticStatus.state === "downloading"
                          || semanticStatus.state === "validating"
                          || semanticStatus.state === "indexing" ? <LoaderCircle className="is-spinning" size={17} /> : null}
                        {semanticStatus.state === "ready" ? <CircleCheck size={17} /> : null}
                        {semanticStatus.state === "failed" ? <CircleAlert size={17} /> : null}
                        {semanticStatus.state === "notInstalled" ? <HardDriveDownload size={17} /> : null}
                        <div>
                          <strong>{t(`fileSpace.settings.semantic.states.${semanticStatus.state}.title`)}</strong>
                          <span>
                            {semanticStatus.state === "indexing"
                              ? t("fileSpace.settings.semantic.indexProgress", {
                                  indexed: semanticStatus.indexedFiles,
                                  total: semanticStatus.totalFiles,
                                })
                              : t(`fileSpace.settings.semantic.states.${semanticStatus.state}.description`)}
                          </span>
                        </div>
                      </div>

                      {semanticStatus.state === "downloading" || semanticStatus.state === "validating" ? (
                        <div className="file-space-settings-semantic-progress">
                          <div
                            role="progressbar"
                            aria-label={t("fileSpace.settings.semantic.downloadProgressLabel")}
                            aria-valuemin={0}
                            aria-valuemax={semanticStatus.totalDownloadBytes}
                            aria-valuenow={semanticStatus.downloadedBytes}
                          >
                            <span style={{ width: `${Math.min(100, Math.max(0, semanticStatus.downloadedBytes / semanticStatus.totalDownloadBytes * 100))}%` }} />
                          </div>
                          <small>{t("fileSpace.settings.semantic.downloadProgress", {
                            downloaded: formatBackupSize(semanticStatus.downloadedBytes),
                            total: formatBackupSize(semanticStatus.totalDownloadBytes),
                          })}</small>
                        </div>
                      ) : null}

                      {semanticStatus.error ? (
                        <small className="file-space-settings-semantic-error">{semanticStatus.error}</small>
                      ) : null}

                      {semanticStatus.installed && !confirmingModelRemoval ? (
                        <div className="file-space-settings-semantic-model-actions">
                          {semanticStatus.state === "failed" ? (
                            <button
                              type="button"
                              disabled={semanticBusy}
                              onClick={() => void runSemanticAction("retry_semantic_search_index")}
                            >
                              <RotateCcw size={14} />
                              {t("fileSpace.settings.semantic.retryIndex")}
                            </button>
                          ) : null}
                          <button
                            className="is-danger"
                            type="button"
                            disabled={semanticBusy}
                            onClick={() => setConfirmingModelRemoval(true)}
                          >
                            <Trash2 size={14} />
                            {t("fileSpace.settings.semantic.removeAction")}
                          </button>
                        </div>
                      ) : null}
                    </>
                  )}
                </section>

                {confirmingModelRemoval ? (
                  <div className="file-space-settings-semantic-removal" role="alert">
                    <CircleAlert size={19} />
                    <div>
                      <strong>{t("fileSpace.settings.semantic.removeTitle")}</strong>
                      <p>{t("fileSpace.settings.semantic.removeDescription")}</p>
                    </div>
                  </div>
                ) : (
                  <div className="file-space-settings-semantic-privacy">
                    <ShieldCheck size={17} />
                    <span>{t("fileSpace.settings.semantic.localNotice")}</span>
                  </div>
                )}
              </div>
            ) : null}

            {activePanel === "feedback" ? <FileSpaceFeedback /> : null}

            {activePanel === "privacy" ? (
              <article className="file-space-settings-privacy">
                <header className="file-space-settings-privacy-intro">
                  <span aria-hidden="true"><ShieldCheck size={20} /></span>
                  <div>
                    <h3>{t("fileSpace.settings.privacyDetails.introTitle")}</h3>
                    <p>{t("fileSpace.settings.privacyDetails.introDescription")}</p>
                  </div>
                </header>

                <div className="file-space-settings-privacy-sections">
                  <section>
                    <h3>{t("fileSpace.settings.privacyDetails.localTitle")}</h3>
                    <p>{t("fileSpace.settings.privacyDetails.localDescription")}</p>
                  </section>

                  <section>
                    <h3>{t("fileSpace.settings.privacyDetails.networkTitle")}</h3>
                    <ul>
                      {(["cloud", "localModel", "agentCli", "modelDownload"] as const).map((item) => (
                        <li key={item}>
                          <strong>{t(`fileSpace.settings.privacyDetails.network.${item}.title`)}</strong>
                          <span>{t(`fileSpace.settings.privacyDetails.network.${item}.description`)}</span>
                        </li>
                      ))}
                    </ul>
                  </section>

                  <section>
                    <h3>{t("fileSpace.settings.privacyDetails.credentialsTitle")}</h3>
                    <p>{t("fileSpace.settings.privacyDetails.credentialsDescription")}</p>
                  </section>

                  <section>
                    <h3>{t("fileSpace.settings.privacyDetails.backupTitle")}</h3>
                    <p>{t("fileSpace.settings.privacyDetails.backupDescription")}</p>
                  </section>
                </div>

                <p className="file-space-settings-privacy-third-party">
                  {t("fileSpace.settings.privacyDetails.thirdPartyDescription")}
                </p>
                <time dateTime="2026-09-04">
                  {t("fileSpace.settings.privacyDetails.updated", {
                    date: new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: "long" }).format(privacyUpdatedAt),
                  })}
                </time>
              </article>
            ) : null}

            {activePanel === "backup" ? (
              <div
                className={`file-space-settings-backup file-space-settings-state is-${backupFeedback.status}`}
                role={backupFeedback.status === "error" ? "alert" : "status"}
                aria-live={backupFeedback.status === "error" ? "assertive" : "polite"}
              >
                {backupFeedback.status === "exporting" ? <LoaderCircle className="is-spinning" size={24} /> : null}
                {backupFeedback.status === "success" ? <CircleCheck size={24} /> : null}
                {backupFeedback.status === "error" ? <CircleAlert size={24} /> : null}
                <div>
                  <strong>
                    {backupFeedback.status === "exporting" ? t("fileSpace.settings.backupExportingTitle") : null}
                    {backupFeedback.status === "success" ? t("fileSpace.settings.backupSuccessTitle") : null}
                    {backupFeedback.status === "error" ? t("fileSpace.settings.backupFailureTitle") : null}
                  </strong>
                  <p>
                    {backupFeedback.status === "exporting" ? t("fileSpace.settings.backupExportingDescription") : null}
                    {backupFeedback.status === "success" ? t("fileSpace.settings.backupSuccessDescription") : null}
                    {backupFeedback.status === "error" ? t("fileSpace.settings.backupFailureDescription") : null}
                  </p>
                  {backupFeedback.path ? <code title={backupFeedback.path}>{backupFeedback.path}</code> : null}
                  {backupFeedback.error ? <small>{backupFeedback.error}</small> : null}
                </div>
              </div>
            ) : null}

            {activePanel === "restore" ? (
              <div
                className={`file-space-settings-restore file-space-settings-state is-${restoreFeedback.status}`}
                role={restoreFeedback.status === "error" ? "alert" : "status"}
                aria-live={restoreFeedback.status === "error" ? "assertive" : "polite"}
              >
                {restoreFeedback.status === "ready" ? <ArchiveRestore size={24} /> : null}
                {restoreFeedback.status === "restoring" ? <LoaderCircle className="is-spinning" size={24} /> : null}
                {restoreFeedback.status === "success" ? <CircleCheck size={24} /> : null}
                {restoreFeedback.status === "error" ? <CircleAlert size={24} /> : null}
                <div>
                  <strong>
                    {restoreFeedback.status === "ready" ? t("fileSpace.settings.restoreReadyTitle") : null}
                    {restoreFeedback.status === "restoring" ? t("fileSpace.settings.restoreRestoringTitle") : null}
                    {restoreFeedback.status === "success" ? t("fileSpace.settings.restoreSuccessTitle") : null}
                    {restoreFeedback.status === "error" ? t("fileSpace.settings.restoreFailureTitle") : null}
                  </strong>
                  <p>
                    {restoreFeedback.status === "ready" ? t("fileSpace.settings.restoreWarning") : null}
                    {restoreFeedback.status === "restoring" ? t("fileSpace.settings.restoreRestoringDescription") : null}
                    {restoreFeedback.status === "success" ? t("fileSpace.settings.restoreSuccessDescription") : null}
                    {restoreFeedback.status === "error" ? t("fileSpace.settings.restoreFailureDescription") : null}
                  </p>
                  {restoreFeedback.status === "ready" && restoreFeedback.inspection ? (
                    <dl>
                      <div>
                        <dt>{t("fileSpace.settings.restoreBackupDate")}</dt>
                        <dd>{new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: "medium", timeStyle: "short" }).format(restoreFeedback.inspection.exportedAt)}</dd>
                      </div>
                      <div>
                        <dt>{t("fileSpace.settings.restoreBackupContents")}</dt>
                        <dd>{t("fileSpace.settings.restoreBackupContentsValue", {
                          count: restoreFeedback.inspection.workspaceFileCount,
                          size: formatBackupSize(restoreFeedback.inspection.workspaceSizeBytes),
                        })}</dd>
                      </div>
                      <div>
                        <dt>{t("fileSpace.settings.restoreFolderName")}</dt>
                        <dd>{restoreFeedback.inspection.workspaceName}</dd>
                      </div>
                    </dl>
                  ) : null}
                  {restoreFeedback.status === "ready" && restoreFeedback.destinationDirectory ? (
                    <code title={restoreFeedback.destinationDirectory}>{restoreFeedback.destinationDirectory}</code>
                  ) : null}
                  {restoreFeedback.restoredRootPath ? (
                    <code title={restoreFeedback.restoredRootPath}>{restoreFeedback.restoredRootPath}</code>
                  ) : null}
                  {restoreFeedback.error ? <small>{restoreFeedback.error}</small> : null}
                </div>
              </div>
            ) : null}

            {activePanel !== "workspace" && activePanel !== "preferences" ? <footer>
              {activePanel === "feedback" ? (
                <button type="button" onClick={closeDialog}>{t("fileSpace.settings.close")}</button>
              ) : activePanel === "semantic" && confirmingModelRemoval ? (
                <>
                  <button type="button" disabled={semanticBusy} onClick={() => setConfirmingModelRemoval(false)}>
                    {t("fileSpace.settings.cancel")}
                  </button>
                  <button
                    className="is-danger"
                    type="button"
                    disabled={semanticBusy}
                    onClick={() => void runSemanticAction("remove_semantic_search_model")}
                  >
                    {t("fileSpace.settings.semantic.confirmRemove")}
                  </button>
                </>
              ) : activePanel === "semantic" && semanticView !== "status" ? (
                <button className="is-primary" type="button" onClick={closeDialog}>{t("fileSpace.settings.done")}</button>
              ) : activePanel === "semantic" && (semanticStatus.state === "downloading" || semanticStatus.state === "validating") ? (
                <button
                  type="button"
                  disabled={semanticBusy}
                  onClick={() => void runSemanticAction("cancel_semantic_search_model_download")}
                >
                  {t("fileSpace.settings.semantic.cancelDownload")}
                </button>
              ) : activePanel === "semantic" && !semanticStatus.installed ? (
                <>
                  <button type="button" onClick={closeDialog}>{t("fileSpace.settings.cancel")}</button>
                  <button
                    className="is-primary"
                    type="button"
                    disabled={semanticBusy}
                    onClick={() => void runSemanticAction("install_semantic_search_model")}
                  >
                    {semanticBusy ? t("fileSpace.settings.semantic.startingDownload") : t("fileSpace.settings.semantic.downloadAction")}
                  </button>
                </>
              ) : activePanel === "restore" && restoreFeedback.status === "ready" ? (
                <>
                  <button type="button" onClick={closeDialog}>{t("fileSpace.settings.cancel")}</button>
                  <button className="is-danger" type="button" onClick={() => void confirmRestoreBackup()}>
                    {t("fileSpace.settings.restoreAction")}
                  </button>
                </>
              ) : activePanel === "restore" && restoreFeedback.status === "restoring" ? (
                <button className="is-primary" type="button" disabled>{t("fileSpace.settings.restoreRestoringAction")}</button>
              ) : (
                <button className="is-primary" type="button" onClick={closeDialog}>{t("fileSpace.settings.done")}</button>
              )}
            </footer> : null}
          </section>
        </div>,
        document.body,
      ) : null}
    </div>
  );
}
