import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ChevronLeft,
  Folder,
  FolderPlus,
  LoaderCircle,
  Pencil,
  Plus,
  Trash2,
  Users,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWorkspaceExtension } from "../../shared/extensions/ApplicationExtension";

export interface FileSpaceWorkspaceRecord {
  id: string;
  name: string;
  kind: "local" | "team";
  rootPath: string | null;
  memberCount: number;
  current: boolean;
  createdAt: number;
  lastOpenedAt: number;
}

export interface FileSpaceWorkspaceDirectory {
  currentWorkspaceId: string;
  workspaces: FileSpaceWorkspaceRecord[];
}

export interface FileSpaceWorkspaceMutation<TSnapshot> {
  directory: FileSpaceWorkspaceDirectory;
  snapshot: TSnapshot;
}

interface FileSpaceWorkspaceStatusProps {
  directory: FileSpaceWorkspaceDirectory | null;
}

interface FileSpaceWorkspaceSettingsProps<TSnapshot> {
  directory: FileSpaceWorkspaceDirectory | null;
  disabled?: boolean;
  onWorkspaceChanged: (mutation: FileSpaceWorkspaceMutation<TSnapshot>) => void;
  onDirectoryChanged: (directory: FileSpaceWorkspaceDirectory) => void;
  onBusyChange?: (busy: boolean) => void;
  onDone: () => void;
}

type WorkspaceCreationMode = "new" | "import";
type WorkspaceSettingsView = "overview" | "create";
type WorkspaceBusyAction = "switch" | "create" | "rename" | "remove";

interface WorkspaceImportProgress {
  requestId: string;
  phase: "scanning" | "importing" | "completed";
  processed: number;
  total: number;
  currentName: string | null;
}

function errorText(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error ?? "");
}

function waitForCommittedPaint() {
  return new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => window.requestAnimationFrame(() => resolve()));
  });
}

function createWorkspaceRequestId() {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `workspace-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function FileSpaceWorkspaceStatus({ directory }: FileSpaceWorkspaceStatusProps) {
  const { t } = useTranslation();
  const external = useWorkspaceExtension();
  const currentWorkspace = directory?.workspaces.find(
    (workspace) => workspace.id === directory.currentWorkspaceId,
  );

  if (external?.active) return <div className="file-space-workspace-status"><span aria-hidden="true" /><span><strong>{external.name}</strong><small>{external.typeLabel}</small></span></div>;
  if (!currentWorkspace) return <div className="file-space-workspace-status-placeholder" />;

  return (
    <div className="file-space-workspace-status" title={currentWorkspace.rootPath ?? undefined}>
      <span className="file-space-health-dot" aria-hidden="true" />
      <span>
        <strong>{currentWorkspace.name}</strong>
        <small>
          {currentWorkspace.kind === "team"
            ? t("fileSpace.workspaces.teamStatus")
            : t("fileSpace.workspaces.localStatus")}
        </small>
      </span>
    </div>
  );
}

export function FileSpaceWorkspaceSettings<TSnapshot>({
  directory,
  disabled = false,
  onWorkspaceChanged,
  onDirectoryChanged,
  onBusyChange,
  onDone,
}: FileSpaceWorkspaceSettingsProps<TSnapshot>) {
  const { t } = useTranslation();
  const external = useWorkspaceExtension();
  const ExternalList = external?.List;
  const nameInputRef = useRef<HTMLInputElement>(null);
  const creationRequestRef = useRef<string | null>(null);
  const [view, setView] = useState<WorkspaceSettingsView>("overview");
  const [creationMode, setCreationMode] = useState<WorkspaceCreationMode>("new");
  const [workspaceName, setWorkspaceName] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [removingWorkspace, setRemovingWorkspace] = useState<FileSpaceWorkspaceRecord | null>(null);
  const [deleteWorkspaceData, setDeleteWorkspaceData] = useState(false);
  const [busyAction, setBusyAction] = useState<WorkspaceBusyAction | null>(null);
  const [switchingWorkspaceId, setSwitchingWorkspaceId] = useState<string | null>(null);
  const [creationProgress, setCreationProgress] = useState<WorkspaceImportProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const localWorkspaces = useMemo(
    () => directory?.workspaces.filter((workspace) => workspace.kind === "local").map(workspace => ({ ...workspace, current: workspace.current && !external?.active })) ?? [],
    [directory, external?.active],
  );
  const teamWorkspaces = useMemo(
    () => directory?.workspaces.filter((workspace) => workspace.kind === "team") ?? [],
    [directory],
  );

  useEffect(() => {
    onBusyChange?.(Boolean(busyAction));
  }, [busyAction, onBusyChange]);

  useEffect(() => {
    if (!isTauri()) return undefined;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<WorkspaceImportProgress>("file-space-import-progress", ({ payload }) => {
      if (disposed || payload.requestId !== creationRequestRef.current) return;
      setCreationProgress(payload);
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
    if (view !== "create") return;
    window.requestAnimationFrame(() => nameInputRef.current?.focus());
  }, [view]);

  const openCreateView = () => {
    setCreationMode("new");
    setWorkspaceName("");
    setWorkspacePath("");
    setRemovingWorkspace(null);
    setRenamingId(null);
    setCreationProgress(null);
    setError(null);
    setView("create");
  };

  const closeCreateView = () => {
    if (busyAction) return;
    setError(null);
    setView("overview");
  };

  const chooseWorkspacePath = async () => {
    try {
      const selected = await open({
        title: creationMode === "import"
          ? t("fileSpace.workspaces.chooseImportFolder")
          : t("fileSpace.workspaces.chooseStorageFolder"),
        directory: true,
        multiple: false,
      });
      if (typeof selected !== "string") return;
      setWorkspacePath(selected);
      if (!workspaceName.trim()) {
        const fallbackName = selected.split(/[\\/]/).filter(Boolean).pop() ?? "";
        setWorkspaceName(fallbackName);
      }
      setError(null);
    } catch (chooseError) {
      setError(errorText(chooseError));
    }
  };

  const switchWorkspace = async (workspace: FileSpaceWorkspaceRecord) => {
    if (workspace.current || busyAction || disabled) return;
    if (external?.active && workspace.id === directory?.currentWorkspaceId) {
      onDone();
      external.onLocalSelect();
      return;
    }
    setBusyAction("switch");
    setSwitchingWorkspaceId(workspace.id);
    setRemovingWorkspace(null);
    setRenamingId(null);
    setError(null);
    try {
      await waitForCommittedPaint();
      const mutation = await invoke<FileSpaceWorkspaceMutation<TSnapshot>>(
        "switch_file_space_workspace",
        { workspaceId: workspace.id },
      );
      onWorkspaceChanged(mutation);
      if (external?.active) { onDone(); external.onLocalSelect(); }
    } catch (switchError) {
      setError(`${t("fileSpace.workspaces.errors.switch")} ${errorText(switchError)}`);
    } finally {
      setSwitchingWorkspaceId(null);
      setBusyAction(null);
    }
  };

  const createWorkspace = async () => {
    const name = workspaceName.trim();
    if (!name || !workspacePath || busyAction) return;
    const requestId = createWorkspaceRequestId();
    creationRequestRef.current = requestId;
    setBusyAction("create");
    setCreationProgress(creationMode === "import" ? {
      requestId,
      phase: "scanning",
      processed: 0,
      total: 0,
      currentName: null,
    } : null);
    setError(null);
    try {
      await waitForCommittedPaint();
      const mutation = await invoke<FileSpaceWorkspaceMutation<TSnapshot>>(
        "create_file_space_workspace",
        { request: { requestId, name, path: workspacePath, mode: creationMode } },
      );
      onWorkspaceChanged(mutation);
      if (external?.active) { onDone(); external.onLocalSelect(); }
      setView("overview");
    } catch (createError) {
      setError(`${t("fileSpace.workspaces.errors.create")} ${errorText(createError)}`);
    } finally {
      if (creationRequestRef.current === requestId) creationRequestRef.current = null;
      setCreationProgress(null);
      setBusyAction(null);
    }
  };

  const beginRename = (workspace: FileSpaceWorkspaceRecord) => {
    setRenamingId(workspace.id);
    setRenameDraft(workspace.name);
    setRemovingWorkspace(null);
    setError(null);
  };

  const saveRename = async () => {
    const name = renameDraft.trim();
    if (!renamingId || !name || busyAction) return;
    setBusyAction("rename");
    setError(null);
    try {
      const next = await invoke<FileSpaceWorkspaceDirectory>("rename_file_space_workspace", {
        workspaceId: renamingId,
        name,
      });
      onDirectoryChanged(next);
      setRenamingId(null);
    } catch (renameError) {
      setError(`${t("fileSpace.workspaces.errors.rename")} ${errorText(renameError)}`);
    } finally {
      setBusyAction(null);
    }
  };

  const removeWorkspace = async () => {
    if (!removingWorkspace || busyAction) return;
    setBusyAction("remove");
    setError(null);
    try {
      const next = await invoke<FileSpaceWorkspaceDirectory>("remove_file_space_workspace", {
        request: {
          workspaceId: removingWorkspace.id,
          deleteWorkspaceData,
        },
      });
      onDirectoryChanged(next);
      setRemovingWorkspace(null);
      setDeleteWorkspaceData(false);
    } catch (removeError) {
      setError(`${t("fileSpace.workspaces.errors.remove")} ${errorText(removeError)}`);
    } finally {
      setBusyAction(null);
    }
  };

  const renderWorkspaceGroup = (label: string, workspaces: FileSpaceWorkspaceRecord[]) => {
    if (workspaces.length === 0) return null;
    return (
      <section className="file-space-workspace-settings-group">
        <span>{label}</span>
        <div role="radiogroup" aria-label={label}>
          {workspaces.map((workspace) => (
            <div
              className={`file-space-workspace-settings-row${workspace.current ? " is-current" : ""}`}
              key={workspace.id}
              onClick={(event) => {
                const target = event.target;
                if (target instanceof Element && target.closest("button, input")) return;
                void switchWorkspace(workspace);
              }}
            >
              <span className={`file-space-workspace-icon is-${workspace.kind}`} aria-hidden="true">
                {workspace.kind === "team" ? <Users size={16} /> : <Folder size={16} />}
              </span>
              {renamingId === workspace.id ? (
                <input
                  value={renameDraft}
                  maxLength={80}
                  onChange={(event) => setRenameDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void saveRename();
                    if (event.key === "Escape") setRenamingId(null);
                  }}
                  autoFocus
                />
              ) : (
                <button
                  className="file-space-workspace-settings-select"
                  type="button"
                  role="radio"
                  aria-checked={workspace.current}
                  aria-busy={switchingWorkspaceId === workspace.id}
                  disabled={workspace.current || Boolean(busyAction) || disabled}
                  onClick={() => void switchWorkspace(workspace)}
                >
                  <span>
                    <strong>{workspace.name}</strong>
                    <small title={workspace.rootPath ?? undefined}>
                      {workspace.kind === "team"
                        ? t("fileSpace.workspaces.teamMeta", { count: workspace.memberCount })
                        : workspace.rootPath ?? t("fileSpace.workspaces.notConfigured")}
                    </small>
                  </span>
                  {switchingWorkspaceId === workspace.id ? (
                    <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                  ) : workspace.current ? <Check size={16} aria-hidden="true" /> : null}
                </button>
              )}
              <div className="file-space-workspace-settings-row-actions">
                {renamingId === workspace.id ? (
                  <>
                    <button type="button" disabled={Boolean(busyAction)} onClick={() => setRenamingId(null)}>
                      {t("fileSpace.settings.cancel")}
                    </button>
                    <button type="button" disabled={!renameDraft.trim() || Boolean(busyAction)} onClick={() => void saveRename()}>
                      {t("fileSpace.workspaces.save")}
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      disabled={Boolean(busyAction)}
                      onClick={() => beginRename(workspace)}
                      aria-label={`${t("fileSpace.workspaces.rename")} ${workspace.name}`}
                      title={t("fileSpace.workspaces.rename")}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="is-danger"
                      type="button"
                      disabled={workspace.id === directory?.currentWorkspaceId || Boolean(busyAction)}
                      onClick={() => {
                        setRemovingWorkspace(workspace);
                        setDeleteWorkspaceData(false);
                        setRenamingId(null);
                      }}
                      aria-label={`${t("fileSpace.workspaces.remove")} ${workspace.name}`}
                      title={t("fileSpace.workspaces.remove")}
                    >
                      <Trash2 size={14} />
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      </section>
    );
  };

  if (view === "create") {
    return (
      <div className="file-space-workspace-settings is-create">
        <div className="file-space-workspace-settings-subheading">
          <button type="button" disabled={Boolean(busyAction)} onClick={closeCreateView} aria-label={t("fileSpace.workspaces.manageTitle")}>
            <ChevronLeft size={17} />
          </button>
          <div>
            <strong>{t("fileSpace.workspaces.createTitle")}</strong>
            <span>{t("fileSpace.workspaces.createDescription")}</span>
          </div>
        </div>
        <div className="file-space-workspace-create-form" aria-busy={busyAction === "create"}>
          <div className="file-space-workspace-mode" role="radiogroup" aria-label={t("fileSpace.workspaces.creationMode")}>
            <button type="button" role="radio" aria-checked={creationMode === "new"} className={creationMode === "new" ? "is-active" : undefined} disabled={Boolean(busyAction)} onClick={() => setCreationMode("new")}>
              <FolderPlus size={16} />{t("fileSpace.workspaces.modeNew")}
            </button>
            <button type="button" role="radio" aria-checked={creationMode === "import"} className={creationMode === "import" ? "is-active" : undefined} disabled={Boolean(busyAction)} onClick={() => setCreationMode("import")}>
              <Folder size={16} />{t("fileSpace.workspaces.modeImport")}
            </button>
          </div>
          <label>
            <span>{t("fileSpace.workspaces.name")}</span>
            <input ref={nameInputRef} value={workspaceName} maxLength={80} disabled={Boolean(busyAction)} onChange={(event) => setWorkspaceName(event.target.value)} placeholder={t("fileSpace.workspaces.namePlaceholder")} />
          </label>
          <label>
            <span>{t("fileSpace.workspaces.storageLocation")}</span>
            <div>
              <input value={workspacePath} readOnly disabled={Boolean(busyAction)} placeholder={t("fileSpace.workspaces.noFolderSelected")} />
              <button type="button" disabled={Boolean(busyAction)} onClick={() => void chooseWorkspacePath()}>{t("fileSpace.workspaces.choose")}</button>
            </div>
          </label>
          <p>{creationMode === "import" ? t("fileSpace.workspaces.importNotice") : t("fileSpace.workspaces.isolationNotice")}</p>
          {busyAction === "create" && creationMode === "import" ? (
            <div className="file-space-workspace-import-progress" role="status" aria-live="polite">
              <div>
                <span>
                  {creationProgress?.total
                    ? t("fileSpace.importFeedback.importing")
                    : t("fileSpace.importFeedback.preparing")}
                </span>
                {creationProgress?.total ? (
                  <strong>{creationProgress.processed} / {creationProgress.total}</strong>
                ) : null}
              </div>
              <progress
                max={creationProgress?.total || undefined}
                value={creationProgress?.total ? creationProgress.processed : undefined}
                aria-label={creationProgress?.total
                  ? `${creationProgress.processed} / ${creationProgress.total}`
                  : t("fileSpace.importFeedback.preparing")}
              />
            </div>
          ) : null}
          {error ? <small className="file-space-workspace-error" role="alert">{error}</small> : null}
        </div>
        <footer className="file-space-workspace-settings-footer">
          <button type="button" disabled={Boolean(busyAction)} onClick={closeCreateView}>{t("fileSpace.settings.cancel")}</button>
          <button className="is-primary" type="button" disabled={!workspaceName.trim() || !workspacePath || Boolean(busyAction) || !isTauri()} onClick={() => void createWorkspace()}>
            {busyAction === "create" ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : null}
            <span aria-live="polite">
              {busyAction === "create" ? t("fileSpace.workspaces.creating") : t("fileSpace.workspaces.confirmCreate")}
            </span>
          </button>
        </footer>
      </div>
    );
  }

  return (
    <div className="file-space-workspace-settings">
      <div className="file-space-workspace-settings-heading">
        <div>
          <strong>{t("fileSpace.workspaces.manageTitle")}</strong>
          <span>{t("fileSpace.workspaces.isolationNotice")}</span>
        </div>
        <button type="button" disabled={Boolean(busyAction) || disabled} onClick={openCreateView}>
          <Plus size={15} />
          {t("fileSpace.workspaces.create")}
        </button>
      </div>
      <div className="file-space-workspace-settings-list">
        {renderWorkspaceGroup(t("fileSpace.workspaces.localSection"), localWorkspaces)}
        {renderWorkspaceGroup(t("fileSpace.workspaces.teamSection"), teamWorkspaces)}
        {ExternalList ? <ExternalList disabled={Boolean(busyAction) || disabled} onDone={onDone} /> : null}
      </div>
      {removingWorkspace ? (
        <div className="file-space-workspace-remove-confirmation" role="alertdialog" aria-modal="false" aria-labelledby="file-space-remove-workspace-title">
          <div className="file-space-workspace-remove-heading">
            <span aria-hidden="true"><Trash2 size={18} /></span>
            <div>
              <strong id="file-space-remove-workspace-title">{t("fileSpace.workspaces.removeTitle", { name: removingWorkspace.name })}</strong>
              <p>{t("fileSpace.workspaces.removeDescription")}</p>
            </div>
          </div>
          <div className="file-space-workspace-remove-options">
            <label className="is-required">
              <input type="checkbox" checked disabled />
              <span>
                <strong>{t("fileSpace.workspaces.removeRecord")}</strong>
                <small>{t("fileSpace.workspaces.removeRecordDescription")}</small>
              </span>
            </label>
            <label className={deleteWorkspaceData ? "is-selected" : undefined}>
              <input
                type="checkbox"
                checked={deleteWorkspaceData}
                disabled={Boolean(busyAction)}
                onChange={(event) => setDeleteWorkspaceData(event.target.checked)}
              />
              <span>
                <strong>{t("fileSpace.workspaces.deleteManagedData")}</strong>
                <small>{t("fileSpace.workspaces.deleteManagedDataDescription")}</small>
              </span>
            </label>
            {deleteWorkspaceData ? (
              <ul>
                <li>{t("fileSpace.workspaces.deleteManagedDatabase")}</li>
                <li>{t("fileSpace.workspaces.deleteManagedIndexes")}</li>
                <li>{t("fileSpace.workspaces.deleteManagedVersions")}</li>
              </ul>
            ) : null}
          </div>
          <div className="file-space-workspace-remove-preservation">
            <Folder size={16} aria-hidden="true" />
            <span>
              <strong>{t("fileSpace.workspaces.physicalFilesKept")}</strong>
              {removingWorkspace.rootPath ? <small title={removingWorkspace.rootPath}>{removingWorkspace.rootPath}</small> : null}
            </span>
          </div>
          {deleteWorkspaceData ? <p className="file-space-workspace-remove-warning">{t("fileSpace.workspaces.removeIrreversible")}</p> : null}
          <div className="file-space-workspace-remove-actions">
            <button type="button" disabled={Boolean(busyAction)} onClick={() => {
              setRemovingWorkspace(null);
              setDeleteWorkspaceData(false);
            }}>{t("fileSpace.settings.cancel")}</button>
            <button className="is-danger" type="button" disabled={Boolean(busyAction)} onClick={() => void removeWorkspace()}>
              {busyAction === "remove" ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : null}
              {deleteWorkspaceData ? t("fileSpace.workspaces.confirmDeleteManagedData") : t("fileSpace.workspaces.confirmRemove")}
            </button>
          </div>
        </div>
      ) : null}
      {error ? <small className="file-space-workspace-error" role="alert">{error}</small> : null}
      <footer className="file-space-workspace-settings-footer">
        <span />
        <button className="is-primary" type="button" disabled={Boolean(busyAction)} onClick={onDone}>
          {t("fileSpace.settings.done")}
        </button>
      </footer>
    </div>
  );
}
