import { invoke, isTauri } from "@tauri-apps/api/core";
import { CircleAlert, LoaderCircle, Pause } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openBackgroundStatusEventName } from "./aiAnswerPresentation";
import {
  backgroundStatusCount,
  backgroundStatusIndicatorState,
  type BackgroundStatus,
} from "./backgroundStatusPresentation";

export function FileSpaceBackgroundStatusButton() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<BackgroundStatus | null>(null);
  const [statusUnavailable, setStatusUnavailable] = useState(false);

  const refresh = useCallback(async () => {
    if (!isTauri() || document.visibilityState === "hidden") return;
    try {
      const nextStatus = await invoke<BackgroundStatus>("get_file_space_background_status");
      setStatus(nextStatus);
      setStatusUnavailable(false);
    } catch {
      setStatusUnavailable(true);
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

  const indicatorState = backgroundStatusIndicatorState(status, statusUnavailable);
  if (!indicatorState) return null;

  const count = status && indicatorState !== "unavailable" ? backgroundStatusCount(status) : 0;
  const label = t(`fileSpace.settings.background.indicator.${indicatorState}`);
  const StatusIcon = indicatorState === "attention" || indicatorState === "unavailable"
    ? CircleAlert
    : indicatorState === "paused"
      ? Pause
      : LoaderCircle;
  const visualState = indicatorState === "unavailable" ? "attention" : indicatorState;

  return (
    <button
      className={`file-space-background-status-button is-${visualState}`}
      type="button"
      title={t("fileSpace.settings.background.indicator.open", { status: label })}
      aria-label={t("fileSpace.settings.background.indicator.open", { status: label })}
      onClick={() => window.dispatchEvent(new CustomEvent(openBackgroundStatusEventName))}
    >
      <StatusIcon className={indicatorState === "running" ? "is-spinning" : ""} size={14} aria-hidden="true" />
      <span>{label}</span>
      {count > 0 ? <small>{count}</small> : null}
    </button>
  );
}
