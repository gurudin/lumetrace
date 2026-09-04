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

export function shouldShowBackgroundStatus(status: BackgroundStatus | null | undefined) {
  return Boolean(status && status.state !== "ready");
}
