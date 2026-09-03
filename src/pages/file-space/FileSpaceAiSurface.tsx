import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChevronDown,
  CircleAlert,
  FileSearch,
  GitCompareArrows,
  History,
  Link2,
  LoaderCircle,
  LockKeyhole,
  Send,
  Sparkles,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import lumeTraceLogo from "../../../src-tauri/icons/icon.png";
import type { AgentCliKey } from "../../shared/brand/AgentCliLogo";
import { usePresence } from "../../shared/ui/usePresence";
import {
  aiAnswerDurationSeconds,
  aiPendingStatusKey,
  aiPendingElapsedSeconds,
  isAiNoSourcesError,
  openBackgroundStatusEventName,
  referencedAiFiles,
  shouldAcceptAiProgress,
  shouldSelectAiSourceFromClickDetail,
  visibleAiAnswer,
  type AiProgress,
  type AiProgressPhase,
  type FileSpaceAiSourceReference,
} from "./aiAnswerPresentation";
import { recordAgentCliRuntimeSuccess } from "./agentCliDetection";
import { isAiServiceConfigured } from "./aiServiceSettingsState";

export type { FileSpaceAiSourceReference } from "./aiAnswerPresentation";

interface FileSpaceAiSurfaceProps {
  onOpenSource: (source: FileSpaceAiSourceReference, action: "select" | "open") => void;
}

interface AgentCliSettings {
  cli: AgentCliKey;
  permission: "readOnly" | "readWrite";
  version?: string;
}

interface LocalLlmSettings {
  provider: "ollama" | "lmStudio";
  baseUrl: string;
  model: string;
}

interface CloudAiSettings {
  provider: "openai";
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
}

interface AiServiceSettingsSnapshot {
  mode: "cloud" | "local" | "agentCli" | null;
  cloud: CloudAiSettings | null;
  local: LocalLlmSettings | null;
  agentCli: AgentCliSettings | null;
}

type AiConfigurationState = "loading" | "configured" | "unconfigured" | "error";
type AiRequestState = "idle" | "asking";
type AiTurnStatus = "pending" | "completed" | "failed";

interface FileSpaceAiTurn {
  id: string;
  question: string;
  answer: string;
  sources: FileSpaceAiSourceReference[];
  status: AiTurnStatus;
  errorCode: string | null;
  durationMs: number | null;
  createdAt: number;
  updatedAt: number;
}

const aiErrorKeys: Record<string, string> = {
  ai_question_empty: "questionEmpty",
  ai_question_too_long: "questionTooLong",
  ai_request_invalid: "requestInvalid",
  ai_service_not_configured: "serviceNotConfigured",
  ai_service_unsupported: "serviceUnsupported",
  ai_read_only_required: "readOnlyRequired",
  ai_search_failed: "searchFailed",
  ai_no_sources: "noSources",
  ai_hermes_unavailable: "hermesUnavailable",
  ai_hermes_timeout: "hermesTimeout",
  ai_hermes_failed: "hermesFailed",
  ai_hermes_empty: "hermesEmpty",
  ai_hermes_output_too_large: "hermesOutputTooLarge",
  ai_codex_unavailable: "codexUnavailable",
  ai_codex_failed: "codexFailed",
  ai_codex_empty: "codexEmpty",
  ai_codex_output_too_large: "codexOutputTooLarge",
  ai_local_llm_unavailable: "localLlmUnavailable",
  ai_local_llm_timeout: "localLlmTimeout",
  ai_local_llm_failed: "localLlmFailed",
  ai_local_llm_empty: "localLlmEmpty",
  ai_local_llm_output_too_large: "localLlmOutputTooLarge",
  ai_cloud_unavailable: "cloudUnavailable",
  ai_cloud_failed: "cloudFailed",
  ai_cloud_empty: "cloudEmpty",
  ai_cloud_output_too_large: "cloudOutputTooLarge",
  ai_history_failed: "historyFailed",
  ai_interrupted: "interrupted",
};

function aiErrorCode(error: unknown) {
  const message = typeof error === "string"
    ? error
    : error instanceof Error
      ? error.message
      : String(error ?? "");
  return Object.keys(aiErrorKeys).find((candidate) => message.includes(candidate)) ?? "ai_generic";
}

function aiErrorTranslationKey(code: string | null | undefined) {
  return `fileSpace.ai.errors.${code && aiErrorKeys[code] ? aiErrorKeys[code] : "generic"}`;
}

interface AiSourcesDisclosureProps {
  turnId: string;
  sources: FileSpaceAiSourceReference[];
  onOpenSource: (source: FileSpaceAiSourceReference, action: "select" | "open") => void;
}

function AiSourcesDisclosure({ turnId, sources, onOpenSource }: AiSourcesDisclosureProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const sourceListId = `file-space-ai-sources-${turnId}`;

  return (
    <section className="file-space-ai-sources" aria-label={t("fileSpace.ai.sourcesTitle")}>
      <button
        className="file-space-ai-sources-toggle"
        type="button"
        aria-expanded={expanded}
        aria-controls={sourceListId}
        onClick={() => setExpanded((current) => !current)}
      >
        <Link2 size={14} aria-hidden="true" />
        <strong>{t("fileSpace.ai.referencedFiles", { count: sources.length })}</strong>
        <ChevronDown className={expanded ? "is-expanded" : ""} size={14} aria-hidden="true" />
      </button>
      {expanded ? (
        <div id={sourceListId} className="file-space-ai-source-list">
          {sources.map((source, index) => (
            <button
              key={`${turnId}-${source.fileId}`}
              type="button"
              onClick={(event) => {
                if (shouldSelectAiSourceFromClickDetail(event.detail)) {
                  onOpenSource(source, "select");
                }
              }}
              onDoubleClick={(event) => {
                event.preventDefault();
                onOpenSource(source, "open");
              }}
              title={source.relativePath}
            >
              <span className="file-space-ai-source-index">{index + 1}</span>
              <strong>{source.fileName}</strong>
              <small className={`file-space-ai-source-evidence is-${source.evidenceRole}`}>
                <span>
                  {t(source.evidenceRole === "primary"
                    ? "fileSpace.ai.primaryEvidence"
                    : "fileSpace.ai.contextEvidence")}
                </span>
                <span aria-hidden="true">·</span>
                <span>{t("fileSpace.ai.sourceCitationCount", { count: source.citationCount })}</span>
              </small>
            </button>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function AiSafeMarkdown({ markdown }: { markdown: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      disallowedElements={["img"]}
      components={{
        a: ({ children, ...props }) => (
          <a {...props} target="_blank" rel="noreferrer noopener">{children}</a>
        ),
      }}
    >
      {markdown}
    </ReactMarkdown>
  );
}

function AiPendingAnswer({
  phase,
  thinking,
  startedAt,
}: {
  phase: AiProgressPhase;
  thinking: string;
  startedAt: number | null;
}) {
  const { t } = useTranslation();
  const [elapsedSeconds, setElapsedSeconds] = useState(() => (
    aiPendingElapsedSeconds(startedAt)
  ));
  const thinkingRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const updateElapsedSeconds = () => {
      setElapsedSeconds(aiPendingElapsedSeconds(startedAt));
    };
    updateElapsedSeconds();
    const interval = window.setInterval(updateElapsedSeconds, 250);
    return () => window.clearInterval(interval);
  }, [startedAt]);

  useEffect(() => {
    const scrollOwner = thinkingRef.current;
    if (scrollOwner) scrollOwner.scrollTop = scrollOwner.scrollHeight;
  }, [thinking]);

  return (
    <section className="file-space-ai-answer is-loading" role="status">
      <span><LoaderCircle className="is-spinning" size={16} /></span>
      <div className="file-space-ai-pending-copy">
        <p>{t(`fileSpace.ai.${aiPendingStatusKey(phase)}`)}</p>
        {thinking ? (
          <div
            className="file-space-ai-thinking file-space-ai-answer-content"
            ref={thinkingRef}
            aria-live="polite"
          >
            <AiSafeMarkdown markdown={thinking} />
          </div>
        ) : null}
      </div>
      <small className="file-space-ai-processing-time">
        {t("fileSpace.ai.waiting", { seconds: elapsedSeconds })}
      </small>
    </section>
  );
}

interface AiNoSourcesAnswerProps {
  onOpenBackgroundStatus: () => void;
}

function AiNoSourcesAnswer({ onOpenBackgroundStatus }: AiNoSourcesAnswerProps) {
  const { t } = useTranslation();

  return (
    <section className="file-space-ai-answer" aria-label={t("fileSpace.ai.answerLabel")}>
      <span className="file-space-ai-answer-mark" aria-hidden="true">
        <img src={lumeTraceLogo} alt="" />
      </span>
      <p>
        {t("fileSpace.ai.noSourcesBefore")}
        <button
          className="file-space-ai-no-sources-link"
          type="button"
          onClick={onOpenBackgroundStatus}
        >
          {t("fileSpace.ai.confirmIndex")}
        </button>
        {t("fileSpace.ai.noSourcesAfter")}
      </p>
    </section>
  );
}

function AiMarkdownAnswer({ answer }: { answer: string }) {
  return (
    <div className="file-space-ai-answer-content">
      <AiSafeMarkdown markdown={visibleAiAnswer(answer)} />
    </div>
  );
}

export function FileSpaceAiSurface({ onOpenSource }: FileSpaceAiSurfaceProps) {
  const { t } = useTranslation();
  const [panelOpen, setPanelOpen] = useState(false);
  const [configurationState, setConfigurationState] = useState<AiConfigurationState>("loading");
  const [serviceSettings, setServiceSettings] = useState<AiServiceSettingsSnapshot | null>(null);
  const [question, setQuestion] = useState("");
  const [requestState, setRequestState] = useState<AiRequestState>("idle");
  const [turns, setTurns] = useState<FileSpaceAiTurn[]>([]);
  const [pendingQuestion, setPendingQuestion] = useState<string | null>(null);
  const [pendingErrorCode, setPendingErrorCode] = useState<string | null>(null);
  const [pendingPhase, setPendingPhase] = useState<AiProgressPhase>("retrieving");
  const [pendingThinking, setPendingThinking] = useState("");
  const [requestStartedAt, setRequestStartedAt] = useState<number | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelCloseRef = useRef<HTMLButtonElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const conversationRef = useRef<HTMLDivElement>(null);
  const requestInFlightRef = useRef(false);
  const activeRequestIdRef = useRef<string | null>(null);
  const shouldFollowConversationRef = useRef(true);
  const panelPresence = usePresence(panelOpen);

  const loadAiServiceSettings = useCallback(async () => {
    if (!isTauri()) {
      setServiceSettings(null);
      setConfigurationState("unconfigured");
      return;
    }
    setConfigurationState("loading");
    try {
      const settings = await invoke<AiServiceSettingsSnapshot>("get_ai_service_settings");
      const configured = isAiServiceConfigured(settings);
      setServiceSettings(settings);
      if (configured) {
        const history = await invoke<FileSpaceAiTurn[]>("get_file_space_ai_history");
        setTurns(history);
      }
      setConfigurationState(configured ? "configured" : "unconfigured");
    } catch {
      setServiceSettings(null);
      setConfigurationState("error");
    }
  }, []);

  useEffect(() => {
    void loadAiServiceSettings();
    const refreshSettings = () => void loadAiServiceSettings();
    window.addEventListener("lumetrace:agent-cli-settings-changed", refreshSettings);
    window.addEventListener("lumetrace:ai-service-settings-changed", refreshSettings);
    return () => {
      window.removeEventListener("lumetrace:agent-cli-settings-changed", refreshSettings);
      window.removeEventListener("lumetrace:ai-service-settings-changed", refreshSettings);
    };
  }, [loadAiServiceSettings]);

  useEffect(() => {
    if (!isTauri()) return undefined;
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listen<AiProgress>("file-space-ai-progress", ({ payload }) => {
      if (!shouldAcceptAiProgress(activeRequestIdRef.current, payload)) return;
      setPendingPhase(payload.phase);
      setPendingThinking(payload.phase === "thinking" ? payload.thinking : "");
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  const restoreTriggerFocus = useCallback(() => {
    window.requestAnimationFrame(() => triggerRef.current?.focus());
  }, []);

  const closePanel = useCallback((restoreFocus = true) => {
    setPanelOpen(false);
    if (restoreFocus) restoreTriggerFocus();
  }, [restoreTriggerFocus]);

  const canAsk = configurationState === "configured"
    && ((serviceSettings?.mode === "cloud"
      && Boolean(serviceSettings.cloud?.hasApiKey && serviceSettings.cloud.model))
      || (serviceSettings?.mode === "local" && Boolean(serviceSettings.local))
      || (serviceSettings?.mode === "agentCli"
        && (serviceSettings.agentCli?.cli === "hermes" || serviceSettings.agentCli?.cli === "codex")
        && serviceSettings.agentCli.permission === "readOnly"));
  const activeServiceLabel = serviceSettings?.mode === "cloud"
    ? serviceSettings.cloud?.model
    : serviceSettings?.mode === "local"
      ? serviceSettings.local?.model
      : serviceSettings?.mode === "agentCli" && serviceSettings.agentCli
      ? t(`fileSpace.settings.aiService.providers.${serviceSettings.agentCli.cli}`)
      : null;

  useEffect(() => {
    if (!panelPresence.mounted || !panelOpen) return undefined;
    const frame = window.requestAnimationFrame(() => {
      if (canAsk) composerRef.current?.focus();
      else panelCloseRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [canAsk, panelOpen, panelPresence.mounted]);

  useEffect(() => {
    if (!panelOpen) return undefined;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (document.querySelector('[aria-modal="true"]')) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      closePanel();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [closePanel, panelOpen]);

  useEffect(() => {
    if (!panelOpen || !shouldFollowConversationRef.current) return undefined;
    let layoutFrame = 0;
    const frame = window.requestAnimationFrame(() => {
      const scrollOwner = conversationRef.current;
      if (!scrollOwner) return;
      scrollOwner.scrollTop = scrollOwner.scrollHeight;
      layoutFrame = window.requestAnimationFrame(() => {
        const settledScrollOwner = conversationRef.current;
        if (!settledScrollOwner || !shouldFollowConversationRef.current) return;
        settledScrollOwner.scrollTo({
          top: settledScrollOwner.scrollHeight,
          behavior: requestState === "asking" && !pendingThinking ? "smooth" : "auto",
        });
      });
    });
    return () => {
      window.cancelAnimationFrame(frame);
      window.cancelAnimationFrame(layoutFrame);
    };
  }, [configurationState, panelOpen, panelPresence.state, pendingErrorCode, pendingPhase, pendingQuestion, pendingThinking, requestState, turns]);

  const updateConversationFollowState = () => {
    const scrollOwner = conversationRef.current;
    if (!scrollOwner) return;
    const distanceFromBottom = scrollOwner.scrollHeight
      - scrollOwner.scrollTop
      - scrollOwner.clientHeight;
    shouldFollowConversationRef.current = distanceFromBottom <= 80;
  };

  const openAiServiceSettings = () => {
    closePanel(false);
    window.dispatchEvent(new CustomEvent("lumetrace:open-ai-service-settings"));
  };

  const openBackgroundStatus = () => {
    closePanel(false);
    window.dispatchEvent(new CustomEvent(openBackgroundStatusEventName));
  };

  const submitQuestion = useCallback(async (
    requestedQuestion?: string,
    retryTurnId?: string,
  ) => {
    const nextQuestion = (requestedQuestion ?? question).trim();
    if (!nextQuestion || requestInFlightRef.current || !canAsk || !isTauri()) return;
    const requestId = crypto.randomUUID();
    requestInFlightRef.current = true;
    activeRequestIdRef.current = requestId;
    setRequestStartedAt(Date.now());
    shouldFollowConversationRef.current = true;
    if (composerRef.current) composerRef.current.value = "";
    setQuestion("");
    setRequestState("asking");
    setPendingErrorCode(null);
    setPendingPhase("retrieving");
    setPendingThinking("");
    if (retryTurnId) {
      setTurns((current) => current.map((turn) => (
        turn.id === retryTurnId
          ? { ...turn, status: "pending", errorCode: null }
          : turn
      )));
    } else {
      setPendingQuestion(nextQuestion);
    }
    try {
      const response = await invoke<FileSpaceAiTurn>("ask_file_space_ai", {
        question: nextQuestion,
        retryTurnId: retryTurnId ?? null,
        requestId,
      });
      if (serviceSettings?.mode === "agentCli" && serviceSettings.agentCli) {
        recordAgentCliRuntimeSuccess(serviceSettings.agentCli.cli);
      }
      setTurns((current) => {
        const existingIndex = current.findIndex((turn) => turn.id === response.id);
        if (existingIndex < 0) return [...current, response].slice(-100);
        return current.map((turn) => turn.id === response.id ? response : turn);
      });
      setPendingQuestion(null);
    } catch (error) {
      const errorCode = aiErrorCode(error);
      if (retryTurnId) {
        setTurns((current) => current.map((turn) => (
          turn.id === retryTurnId
            ? { ...turn, status: "failed", errorCode }
            : turn
        )));
      } else {
        setPendingQuestion(nextQuestion);
        setPendingErrorCode(errorCode);
      }
    } finally {
      requestInFlightRef.current = false;
      activeRequestIdRef.current = null;
      setPendingPhase("retrieving");
      setPendingThinking("");
      setRequestStartedAt(null);
      setRequestState("idle");
    }
  }, [canAsk, question, serviceSettings]);

  const suggestionKeys = [
    { key: "recentChanges", icon: History },
    { key: "projectDecisions", icon: FileSearch },
    { key: "compareVersions", icon: GitCompareArrows },
  ] as const;

  return (
    <>
      <div className="file-space-ai-menu">
        <button
          ref={triggerRef}
          className={`file-space-toolbar-icon-button${panelOpen ? " is-active" : ""}${requestState === "asking" ? " is-ai-running" : ""}`}
          type="button"
          title={t("fileSpace.ai.openWorkspace")}
          aria-label={t("fileSpace.ai.openWorkspace")}
          aria-expanded={panelOpen}
          aria-pressed={panelOpen}
          aria-busy={requestState === "asking"}
          onClick={() => setPanelOpen((open) => {
            if (!open) shouldFollowConversationRef.current = true;
            return !open;
          })}
        >
          {requestState === "asking"
            ? <LoaderCircle className="is-spinning" size={18} />
            : <Sparkles size={18} />}
        </button>
      </div>

      {panelPresence.mounted ? createPortal(
        <aside
          className={`file-space-ai-panel${panelPresence.state === "open" ? " is-open" : ""}`}
          role="dialog"
          aria-modal="false"
          aria-label={t("fileSpace.ai.workspaceTitle")}
          data-ai-status={configurationState}
        >
          <header>
            <strong>{t("fileSpace.ai.workspaceTitle")}</strong>
            {configurationState === "loading" ? (
              <span className="file-space-ai-header-service is-loading" role="status">
                <LoaderCircle className="is-spinning" size={12} />
                {t("fileSpace.ai.configurationLoadingShort")}
              </span>
            ) : configurationState === "configured" && activeServiceLabel ? (
              <button
                className="file-space-ai-header-service is-connected"
                type="button"
                title={t("fileSpace.ai.changeService")}
                aria-label={t("fileSpace.ai.changeService")}
                onClick={openAiServiceSettings}
              >
                <i aria-hidden="true" />
                <span>{activeServiceLabel}</span>
                <ChevronDown size={12} aria-hidden="true" />
              </button>
            ) : null}
            <button className="file-space-ai-close-button" ref={panelCloseRef} type="button" onClick={() => closePanel()} aria-label={t("fileSpace.ai.closePanel")}>
              <X size={17} />
            </button>
          </header>
          {configurationState === "loading" ? (
            <div className="file-space-ai-unavailable is-loading" role="status">
              <span><LoaderCircle className="is-spinning" size={22} /></span>
              <strong>{t("fileSpace.ai.configurationLoading")}</strong>
            </div>
          ) : configurationState === "configured" && serviceSettings ? (
            !canAsk ? (
              <div className="file-space-ai-unavailable is-error" role="alert">
                <span><CircleAlert size={22} /></span>
                <strong>{t("fileSpace.ai.unsupportedConfigurationTitle")}</strong>
                <p>{t("fileSpace.ai.unsupportedConfigurationDescription")}</p>
                <button type="button" onClick={openAiServiceSettings}>
                  {t("fileSpace.ai.changeService")}
                </button>
              </div>
            ) : turns.length > 0 || pendingQuestion ? (
              <div
                className="file-space-ai-conversation"
                ref={conversationRef}
                aria-live="polite"
                onScroll={updateConversationFollowState}
              >
                {turns.map((turn) => (
                  <article className="file-space-ai-turn" key={turn.id}>
                    <section className="file-space-ai-user-message" aria-label={t("fileSpace.ai.questionLabel")}>
                      <p>{turn.question}</p>
                    </section>
                    {turn.status === "pending" ? (
                      <AiPendingAnswer
                        phase={pendingPhase}
                        thinking={pendingThinking}
                        startedAt={requestStartedAt ?? turn.updatedAt}
                      />
                    ) : turn.status === "failed" && isAiNoSourcesError(turn.errorCode) ? (
                      <AiNoSourcesAnswer onOpenBackgroundStatus={openBackgroundStatus} />
                    ) : turn.status === "failed" ? (
                      <section className="file-space-ai-answer-error" role="alert">
                        <CircleAlert size={18} />
                        <div>
                          <strong>{t("fileSpace.ai.answerErrorTitle")}</strong>
                          <p>{t(aiErrorTranslationKey(turn.errorCode))}</p>
                          <button
                            type="button"
                            disabled={requestState === "asking"}
                            onClick={() => void submitQuestion(turn.question, turn.id)}
                          >
                            {t("fileSpace.ai.retryAnswer")}
                          </button>
                        </div>
                      </section>
                    ) : (() => {
                      const referencedFiles = referencedAiFiles(turn.sources);
                      return (
                        <>
                          <section className="file-space-ai-answer" aria-label={t("fileSpace.ai.answerLabel")}>
                            <span className="file-space-ai-answer-mark" aria-hidden="true">
                              <img src={lumeTraceLogo} alt="" />
                            </span>
                            <AiMarkdownAnswer answer={turn.answer} />
                            <small className="file-space-ai-processing-time">
                              {t("fileSpace.ai.processedIn", {
                                seconds: aiAnswerDurationSeconds(
                                  turn.durationMs,
                                  turn.createdAt,
                                  turn.updatedAt,
                                ),
                              })}
                            </small>
                          </section>
                          {referencedFiles.length > 0 ? (
                            <AiSourcesDisclosure
                              turnId={turn.id}
                              sources={referencedFiles}
                              onOpenSource={onOpenSource}
                            />
                          ) : null}
                        </>
                      );
                    })()}
                  </article>
                ))}
                {pendingQuestion ? (
                  <article className="file-space-ai-turn is-transient">
                    <section className="file-space-ai-user-message" aria-label={t("fileSpace.ai.questionLabel")}>
                      <p>{pendingQuestion}</p>
                    </section>
                    {requestState === "asking" ? (
                      <AiPendingAnswer
                        phase={pendingPhase}
                        thinking={pendingThinking}
                        startedAt={requestStartedAt}
                      />
                    ) : isAiNoSourcesError(pendingErrorCode) ? (
                      <AiNoSourcesAnswer onOpenBackgroundStatus={openBackgroundStatus} />
                    ) : (
                      <section className="file-space-ai-answer-error" role="alert">
                        <CircleAlert size={18} />
                        <div>
                          <strong>{t("fileSpace.ai.answerErrorTitle")}</strong>
                          <p>{t(aiErrorTranslationKey(pendingErrorCode))}</p>
                          <button type="button" onClick={() => void submitQuestion(pendingQuestion)}>
                            {t("fileSpace.ai.retryAnswer")}
                          </button>
                        </div>
                      </section>
                    )}
                  </article>
                ) : null}
              </div>
            ) : (
              <div className="file-space-ai-ready" role="status">
                <div className="file-space-ai-ready-copy">
                  <span className="file-space-ai-ready-mark"><FileSearch size={21} /></span>
                  <strong>{t("fileSpace.ai.readyTitle")}</strong>
                  <p>{t("fileSpace.ai.readyDescription")}</p>
                  <div className="file-space-ai-suggestions" aria-label={t("fileSpace.ai.suggestionsLabel")}>
                    {suggestionKeys.map(({ key, icon: SuggestionIcon }) => (
                      <button
                        key={key}
                        type="button"
                        onClick={() => void submitQuestion(t(`fileSpace.ai.suggestions.${key}`))}
                      >
                        <SuggestionIcon size={13} />
                        {t(`fileSpace.ai.suggestions.${key}`)}
                      </button>
                    ))}
                  </div>
                  <small>{t("fileSpace.ai.configuredDescription")}</small>
                </div>
              </div>
            )
          ) : configurationState === "error" ? (
            <div className="file-space-ai-unavailable is-error" role="alert">
              <span><CircleAlert size={22} /></span>
              <strong>{t("fileSpace.ai.configurationErrorTitle")}</strong>
              <p>{t("fileSpace.ai.configurationErrorDescription")}</p>
              <button type="button" onClick={() => void loadAiServiceSettings()}>
                {t("fileSpace.ai.retryConfiguration")}
              </button>
            </div>
          ) : (
            <div className="file-space-ai-unavailable">
              <span><LockKeyhole size={22} /></span>
              <strong>{t("fileSpace.ai.unavailableTitle")}</strong>
              <p>{t("fileSpace.ai.unavailableDescription")}</p>
              <button type="button" onClick={openAiServiceSettings}>
                {t("fileSpace.ai.configureService")}
              </button>
            </div>
          )}
          <div className={`file-space-ai-composer${canAsk ? " is-enabled" : ""}`} aria-disabled={!canAsk}>
            <textarea
              ref={composerRef}
              value={question}
              disabled={!canAsk || requestState === "asking"}
              maxLength={2_000}
              aria-label={t("fileSpace.ai.composerLabel")}
              placeholder={t(canAsk
                ? "fileSpace.ai.configuredComposerPlaceholder"
                : "fileSpace.ai.composerPlaceholder")}
              onChange={(event) => {
                if (!requestInFlightRef.current) setQuestion(event.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
                event.preventDefault();
                void submitQuestion();
              }}
            />
            <button
              type="button"
              disabled={!canAsk || requestState === "asking" || !question.trim()}
              aria-label={t("fileSpace.ai.send")}
              onClick={() => void submitQuestion()}
            >
              {requestState === "asking"
                ? <LoaderCircle className="is-spinning" size={15} />
                : <Send size={15} />}
            </button>
          </div>
          <footer>{t("fileSpace.ai.workspaceNotice")}</footer>
        </aside>,
        document.body,
      ) : null}
    </>
  );
}
