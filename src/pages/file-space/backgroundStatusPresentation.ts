export interface BackgroundPipelineStatus {
  state: "running" | "ready" | "paused" | "failed" | "disabled" | "preparing";
  completedFiles: number;
  totalFiles: number;
  pendingFiles: number;
  failedFiles: number;
  currentFile?: string | null;
  error?: string | null;
}

export interface WatcherStatus {
  state: "starting" | "watching" | "unavailable" | "failed";
  rootPath?: string | null;
  lastCheckedAt?: number | null;
  lastEventAt?: number | null;
  error?: string | null;
}

export interface BackgroundStatus {
  state: "running" | "ready" | "paused" | "attention";
  paused: boolean;
  contentIndex: BackgroundPipelineStatus;
  semanticIndex: BackgroundPipelineStatus;
  semanticModelInstalled: boolean;
  watcher: WatcherStatus;
  updatedAt: number;
}

export type BackgroundStatusIndicatorState = BackgroundStatus["state"] | "unavailable";

export function backgroundStatusCount(status: BackgroundStatus) {
  if (status.state === "attention") {
    return status.contentIndex.failedFiles
      + status.semanticIndex.failedFiles
      + Number(["failed", "unavailable"].includes(status.watcher.state));
  }
  if (status.state === "running") {
    return status.contentIndex.pendingFiles + status.semanticIndex.pendingFiles;
  }
  return 0;
}

export function backgroundStatusIndicatorState(
  status: BackgroundStatus | null | undefined,
  statusUnavailable = false,
): BackgroundStatusIndicatorState | null {
  if (statusUnavailable) return "unavailable";
  if (!status || status.state === "ready") return null;
  return status.state;
}

export function shouldShowBackgroundStatus(
  status: BackgroundStatus | null | undefined,
  statusUnavailable = false,
) {
  return backgroundStatusIndicatorState(status, statusUnavailable) !== null;
}
