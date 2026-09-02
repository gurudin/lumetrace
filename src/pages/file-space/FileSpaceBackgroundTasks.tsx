import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  Activity,
  CircleAlert,
  CircleCheck,
  Eye,
  FileSearch,
  LoaderCircle,
  Pause,
  Play,
  RotateCcw,
  Sparkles,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface BackgroundPipelineStatus {
  state: "running" | "ready" | "paused" | "failed" | "disabled" | "preparing";
  completedFiles: number;
  totalFiles: number;
  pendingFiles: number;
  failedFiles: number;
  currentFile?: string | null;
  error?: string | null;
}

interface WatcherStatus {
  state: "starting" | "watching" | "unavailable" | "failed";
  rootPath?: string | null;
  lastCheckedAt?: number | null;
  lastEventAt?: number | null;
  error?: string | null;
}

interface BackgroundStatus {
  state: "running" | "ready" | "paused" | "attention";
  paused: boolean;
  contentIndex: BackgroundPipelineStatus;
  semanticIndex: BackgroundPipelineStatus;
  semanticModelInstalled: boolean;
  watcher: WatcherStatus;
  updatedAt: number;
}

interface FileSpaceBackgroundTasksProps {
  onOpenSemantic: () => void;
}

function progressPercent(task: BackgroundPipelineStatus) {
  if (task.totalFiles <= 0) return 100;
  return Math.min(100, Math.max(0, task.completedFiles / task.totalFiles * 100));
}

export function FileSpaceBackgroundTasks({ onOpenSemantic }: FileSpaceBackgroundTasksProps) {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<BackgroundStatus | null>(null);
  const [busy, setBusy] = useState<"pause" | "retry" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const formatTime = useMemo(() => new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }), [i18n.resolvedLanguage]);

  const refresh = useCallback(async () => {
    if (!isTauri()) {
      setError(t("fileSpace.settings.background.desktopOnly"));
      return;
    }
    try {
      const next = await invoke<BackgroundStatus>("get_file_space_background_status");
      setStatus(next);
      setError(null);
    } catch (refreshError) {
      setError(refreshError instanceof Error ? refreshError.message : String(refreshError));
    }
  }, [t]);

  useEffect(() => {
    let disposed = false;
    let requestRunning = false;
    const poll = async () => {
      if (requestRunning || disposed || !isTauri()) return;
      requestRunning = true;
      try {
        const next = await invoke<BackgroundStatus>("get_file_space_background_status");
        if (!disposed) {
          setStatus(next);
          setError(null);
        }
      } catch (pollError) {
        if (!disposed) setError(pollError instanceof Error ? pollError.message : String(pollError));
      } finally {
        requestRunning = false;
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), 2_000);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, []);

  const togglePaused = async () => {
    if (!status || busy) return;
    setBusy("pause");
    try {
      const next = await invoke<BackgroundStatus>("set_file_space_background_paused", {
        paused: !status.paused,
      });
      setStatus(next);
      setError(null);
    } catch (actionError) {
      setError(actionError instanceof Error ? actionError.message : String(actionError));
    } finally {
      setBusy(null);
    }
  };

  const retryFailures = async () => {
    if (busy) return;
    setBusy("retry");
    try {
      const next = await invoke<BackgroundStatus>("retry_file_space_background_failures");
      setStatus(next);
      setError(null);
    } catch (actionError) {
      setError(actionError instanceof Error ? actionError.message : String(actionError));
    } finally {
      setBusy(null);
    }
  };

  const failedCount = (status?.contentIndex.failedFiles ?? 0) + (status?.semanticIndex.failedFiles ?? 0);

  const renderPipeline = (
    key: "content" | "semantic",
    task: BackgroundPipelineStatus,
    Icon: typeof FileSearch,
  ) => (
    <section className="file-space-background-task-row">
      <span className={`file-space-background-task-icon is-${task.state}`} aria-hidden="true">
        <Icon size={18} />
      </span>
      <div className="file-space-background-task-main">
        <div className="file-space-background-task-heading">
          <div>
            <strong>{t(`fileSpace.settings.background.tasks.${key}.title`)}</strong>
            <span>{t(`fileSpace.settings.background.tasks.${key}.description`)}</span>
          </div>
          <small className={`is-${task.state}`}>{t(`fileSpace.settings.background.states.${task.state}`)}</small>
        </div>
        {task.state !== "disabled" && task.state !== "preparing" ? (
          <>
            <div className="file-space-background-progress">
              <div
                role="progressbar"
                aria-label={t(`fileSpace.settings.background.tasks.${key}.title`)}
                aria-valuemin={0}
                aria-valuemax={Math.max(0, task.totalFiles)}
                aria-valuenow={Math.max(0, task.completedFiles)}
              >
                <span style={{ width: `${progressPercent(task)}%` }} />
              </div>
              <small>{t("fileSpace.settings.background.progress", {
                completed: task.completedFiles,
                total: task.totalFiles,
                pending: task.pendingFiles,
                failed: task.failedFiles,
              })}</small>
            </div>
            {task.currentFile && task.state === "running" ? (
              <p title={task.currentFile}>{t("fileSpace.settings.background.currentFile", { name: task.currentFile })}</p>
            ) : null}
          </>
        ) : null}
        {key === "semantic" && task.state === "disabled" ? (
          <button type="button" className="file-space-background-inline-action" onClick={onOpenSemantic}>
            {t("fileSpace.settings.background.configureSemantic")}
          </button>
        ) : null}
        {task.error ? <p className="file-space-background-task-error">{task.error}</p> : null}
      </div>
    </section>
  );

  if (!status) {
    return (
      <div className="file-space-settings-background is-loading" aria-live="polite">
        {error ? <CircleAlert size={22} /> : <LoaderCircle className="is-spinning" size={22} />}
        <p>{error ?? t("fileSpace.settings.background.loading")}</p>
        {error && isTauri() ? <button type="button" onClick={() => void refresh()}>{t("fileSpace.settings.background.refresh")}</button> : null}
      </div>
    );
  }

  const SummaryIcon = status.state === "attention"
    ? CircleAlert
    : status.state === "ready"
      ? CircleCheck
      : status.state === "paused"
        ? Pause
        : Activity;

  return (
    <div className="file-space-settings-background" aria-live="polite">
      <section className={`file-space-background-summary is-${status.state}`}>
        <span aria-hidden="true"><SummaryIcon size={20} /></span>
        <div>
          <strong>{t(`fileSpace.settings.background.summary.${status.state}.title`)}</strong>
          <p>{t(`fileSpace.settings.background.summary.${status.state}.description`)}</p>
        </div>
        <button type="button" disabled={Boolean(busy)} onClick={() => void togglePaused()}>
          {busy === "pause" ? <LoaderCircle className="is-spinning" size={14} /> : status.paused ? <Play size={14} /> : <Pause size={14} />}
          {status.paused ? t("fileSpace.settings.background.resume") : t("fileSpace.settings.background.pause")}
        </button>
      </section>

      <div className="file-space-background-task-list">
        {renderPipeline("content", status.contentIndex, FileSearch)}
        {renderPipeline("semantic", status.semanticIndex, Sparkles)}
        <section className="file-space-background-task-row is-watcher">
          <span className={`file-space-background-task-icon is-${status.watcher.state}`} aria-hidden="true">
            <Eye size={18} />
          </span>
          <div className="file-space-background-task-main">
            <div className="file-space-background-task-heading">
              <div>
                <strong>{t("fileSpace.settings.background.tasks.watcher.title")}</strong>
                <span>{t("fileSpace.settings.background.tasks.watcher.description")}</span>
              </div>
              <small className={`is-${status.watcher.state}`}>{t(`fileSpace.settings.background.states.${status.watcher.state}`)}</small>
            </div>
            {status.watcher.rootPath ? <p title={status.watcher.rootPath}>{status.watcher.rootPath}</p> : null}
            <div className="file-space-background-watcher-times">
              {status.watcher.lastCheckedAt ? <small>{t("fileSpace.settings.background.lastChecked", { time: formatTime.format(status.watcher.lastCheckedAt) })}</small> : null}
              {status.watcher.lastEventAt ? <small>{t("fileSpace.settings.background.lastEvent", { time: formatTime.format(status.watcher.lastEventAt) })}</small> : null}
            </div>
            {status.watcher.error ? <p className="file-space-background-task-error">{status.watcher.error}</p> : null}
          </div>
        </section>
      </div>

      <div className="file-space-background-footnote">
        <Eye size={15} aria-hidden="true" />
        <span>{t("fileSpace.settings.background.watcherAlwaysOn")}</span>
      </div>

      {failedCount > 0 ? (
        <div className="file-space-background-retry">
          <span>{t("fileSpace.settings.background.failedSummary", { count: failedCount })}</span>
          <button type="button" disabled={Boolean(busy)} onClick={() => void retryFailures()}>
            {busy === "retry" ? <LoaderCircle className="is-spinning" size={14} /> : <RotateCcw size={14} />}
            {t("fileSpace.settings.background.retry")}
          </button>
        </div>
      ) : null}
      {error ? <small className="file-space-background-panel-error" role="alert">{error}</small> : null}
    </div>
  );
}
