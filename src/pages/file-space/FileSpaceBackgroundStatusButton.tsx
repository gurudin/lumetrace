import { invoke, isTauri } from "@tauri-apps/api/core";
import { CircleAlert, LoaderCircle, Pause } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openBackgroundStatusEventName } from "./aiAnswerPresentation";
import {
  backgroundStatusCount,
  shouldShowBackgroundStatus,
  type BackgroundStatus,
} from "./backgroundStatusPresentation";

export function FileSpaceBackgroundStatusButton() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<BackgroundStatus | null>(null);

  const refresh = useCallback(async () => {
    if (!isTauri() || document.visibilityState === "hidden") return;
    try {
      setStatus(await invoke<BackgroundStatus>("get_file_space_background_status"));
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), 4_000);
    const handleVisibilityChange = () => void refresh();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [refresh]);

  if (!shouldShowBackgroundStatus(status) || !status) return null;

  const count = backgroundStatusCount(status);
  const label = t(`fileSpace.settings.background.indicator.${status.state}`);
  const StatusIcon = status.state === "attention"
    ? CircleAlert
    : status.state === "paused"
      ? Pause
      : LoaderCircle;

  return (
    <button
      className={`file-space-background-status-button is-${status.state}`}
      type="button"
      title={t("fileSpace.settings.background.indicator.open", { status: label })}
      aria-label={t("fileSpace.settings.background.indicator.open", { status: label })}
      onClick={() => window.dispatchEvent(new CustomEvent(openBackgroundStatusEventName))}
    >
      <StatusIcon className={status.state === "running" ? "is-spinning" : ""} size={14} aria-hidden="true" />
      <span>{label}</span>
      {count > 0 ? <small>{count}</small> : null}
    </button>
  );
}
