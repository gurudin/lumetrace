import { invoke, isTauri } from "@tauri-apps/api/core";
import { ExternalLink, Github, LoaderCircle, MessagesSquare } from "lucide-react";
import { useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import channels from "../../shared/feedbackChannels.json";
import "./file-space-feedback.css";

type FeedbackChannel = keyof typeof channels;

export function FileSpaceFeedback() {
  const { t } = useTranslation();
  const openingRef = useRef(false);
  const [opening, setOpening] = useState<FeedbackChannel | null>(null);
  const [failed, setFailed] = useState(false);

  const openChannel = async (event: MouseEvent<HTMLAnchorElement>, channel: FeedbackChannel) => {
    if (openingRef.current) {
      event.preventDefault();
      return;
    }
    if (!isTauri()) return; // Static previews use the same destination in a normal browser tab.
    event.preventDefault();
    openingRef.current = true;
    setOpening(channel);
    setFailed(false);
    try {
      // The native side accepts only a channel, never an arbitrary URL or workspace data.
      await invoke("open_feedback_channel", { channel });
    } catch {
      setFailed(true);
    } finally {
      openingRef.current = false;
      setOpening(null);
    }
  };

  return (
    <div className="file-space-feedback">
      <p>{t("fileSpace.settings.feedbackDetails.description")}</p>
      <div className="file-space-feedback-channels">
        {(["github", "discord"] as const).map((channel) => {
          const url = channels[channel];
          const Icon = channel === "github" ? Github : MessagesSquare;
          const content = <>
            <span className="file-space-feedback-icon" aria-hidden="true"><Icon size={21} /></span>
            <span className="file-space-feedback-copy">
              <strong>{channel === "github" ? "GitHub Issues" : "Discord"}</strong>
              <span>{t(`fileSpace.settings.feedbackDetails.${channel}Description`)}</span>
            </span>
            {!url ? <small>{t("fileSpace.settings.feedbackDetails.unavailable")}</small>
              : opening === channel ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                : <ExternalLink size={16} aria-hidden="true" />}
          </>;
          return url ? (
            <a
              key={channel}
              className="file-space-feedback-channel"
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              aria-disabled={opening !== null}
              aria-busy={opening === channel}
              onClick={(event) => void openChannel(event, channel)}
            >{content}</a>
          ) : (
            <button key={channel} className="file-space-feedback-channel" type="button" disabled>
              {content}
            </button>
          );
        })}
      </div>
      {opening ? <p role="status">{t("fileSpace.settings.feedbackDetails.opening")}</p> : null}
      {failed ? <p className="file-space-feedback-error" role="alert">{t("fileSpace.settings.feedbackDetails.openFailed")}</p> : null}
      <small className="file-space-feedback-notice">{t("fileSpace.settings.feedbackDetails.notice")}</small>
    </div>
  );
}
