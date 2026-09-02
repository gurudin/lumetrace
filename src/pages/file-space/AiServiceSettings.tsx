import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  CircleAlert,
  CircleCheck,
  Cloud,
  HardDrive,
  LoaderCircle,
  LockKeyhole,
  RefreshCw,
  ShieldCheck,
  Terminal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AgentCliLogo,
  agentCliKeys,
  type AgentCliKey,
} from "../../shared/brand/AgentCliLogo";
import { MacSelect, type MacSelectOption } from "../../shared/ui/MacSelect";
import {
  agentCheckState,
  type AgentCheckState,
  type AgentCliStatusResponse,
  wasAgentCliRecentlySuccessful,
} from "./agentCliDetection";
import {
  defaultAiServiceMode,
  defaultLocalBaseUrl,
  type AiServiceMode,
} from "./aiServiceSettingsState";

type CloudProvider = "openaiCompatible";
type LocalProvider = "ollama" | "lmStudio";
type AgentFilePermission = "readOnly" | "readWrite";
type LocalConnectionState = "idle" | "checking" | "passed" | "error";
interface AgentCheck {
  state: AgentCheckState;
  version?: string;
}

interface SavedAgentCliSettings {
  cli: AgentCliKey;
  permission: AgentFilePermission;
  version?: string;
}

interface LocalLlmSettings {
  provider: LocalProvider;
  baseUrl: string;
  model: string;
}

interface LocalLlmConnectionResult {
  baseUrl: string;
  models: string[];
}

interface AiServiceSettingsSnapshot {
  mode: "local" | "agentCli" | null;
  local: LocalLlmSettings | null;
  agentCli: SavedAgentCliSettings | null;
}

interface LegacyAgentCliSettings extends SavedAgentCliSettings {
  mode: "agentCli";
}

interface AiServiceSettingsProps {
  onCancel: () => void;
}

const cloudProviders: readonly CloudProvider[] = ["openaiCompatible"];
const localProviders: readonly LocalProvider[] = ["ollama", "lmStudio"];
const localProviderExampleUrls: Record<LocalProvider, string> = {
  ollama: "http://127.0.0.1:11434/v1",
  lmStudio: "http://127.0.0.1:1234/v1",
};
const agentCliSettingsStorageKey = "lumetrace.aiService.agentCli";
let cachedAgentChecks: Record<AgentCliKey, AgentCheck> | null = null;
let startupAgentCheckPromise: Promise<Record<AgentCliKey, AgentCheck>> | null = null;

function createAgentCheckMap(state: AgentCheckState): Record<AgentCliKey, AgentCheck> {
  return {
    claude: { state },
    hermes: { state },
    codex: { state },
    opencode: { state },
  };
}

function isAgentCliKey(value: unknown): value is AgentCliKey {
  return typeof value === "string" && agentCliKeys.includes(value as AgentCliKey);
}

function isAgentFilePermission(value: unknown): value is AgentFilePermission {
  return value === "readOnly" || value === "readWrite";
}

function readLegacyAgentCliSettings(): SavedAgentCliSettings | null {
  try {
    const value = JSON.parse(localStorage.getItem(agentCliSettingsStorageKey) ?? "null") as Partial<LegacyAgentCliSettings> | null;
    if (!value || value.mode !== "agentCli" || !isAgentCliKey(value.cli) || !isAgentFilePermission(value.permission)) {
      return null;
    }
    return {
      cli: value.cli,
      permission: value.permission,
      version: typeof value.version === "string" ? value.version : undefined,
    };
  } catch {
    return null;
  }
}

function toAgentCheck(result: AgentCliStatusResponse): AgentCheck {
  return {
    state: agentCheckState(result),
    version: result.version ?? undefined,
  };
}

function reconcileRecentRuntimeSuccess(checks: Record<AgentCliKey, AgentCheck>) {
  const successfulAgent = agentCliKeys.find((key) => wasAgentCliRecentlySuccessful(key));
  if (!successfulAgent || checks[successfulAgent].state === "checking") return checks;
  return {
    ...checks,
    [successfulAgent]: { ...checks[successfulAgent], state: "passed" as const },
  };
}

async function requestAgentChecks() {
  if (!isTauri()) return createAgentCheckMap("error");
  try {
    const results = await invoke<AgentCliStatusResponse[]>("check_agent_clis");
    const checks = createAgentCheckMap("missing");
    results.forEach((result) => {
      if (isAgentCliKey(result.key)) checks[result.key] = toAgentCheck(result);
    });
    return reconcileRecentRuntimeSuccess(checks);
  } catch {
    return createAgentCheckMap("error");
  }
}

function getStartupAgentChecks() {
  if (cachedAgentChecks) return Promise.resolve(cachedAgentChecks);
  if (!startupAgentCheckPromise) {
    startupAgentCheckPromise = requestAgentChecks().then((checks) => {
      cachedAgentChecks = checks;
      return checks;
    });
  }
  return startupAgentCheckPromise;
}

export function AiServiceSettings({ onCancel }: AiServiceSettingsProps) {
  const { t } = useTranslation();
  const legacySettings = useMemo(readLegacyAgentCliSettings, []);
  const [mode, setMode] = useState<AiServiceMode>(defaultAiServiceMode);
  const [cloudProvider, setCloudProvider] = useState<CloudProvider>("openaiCompatible");
  const [localProvider, setLocalProvider] = useState<LocalProvider>("ollama");
  const [localBaseUrl, setLocalBaseUrl] = useState(defaultLocalBaseUrl);
  const [localModels, setLocalModels] = useState<string[]>([]);
  const [selectedLocalModel, setSelectedLocalModel] = useState("");
  const [localConnectionState, setLocalConnectionState] = useState<LocalConnectionState>("idle");
  const [selectedAgent, setSelectedAgent] = useState<AgentCliKey>(legacySettings?.cli ?? "claude");
  const [permission, setPermission] = useState<AgentFilePermission>(legacySettings?.permission ?? "readOnly");
  const [agentChecks, setAgentChecks] = useState<Record<AgentCliKey, AgentCheck>>(
    () => reconcileRecentRuntimeSuccess(cachedAgentChecks ?? createAgentCheckMap("idle")),
  );
  const [detectingAgents, setDetectingAgents] = useState(false);
  const [loadingSettings, setLoadingSettings] = useState(() => isTauri());
  const [savingSettings, setSavingSettings] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const requestedStartupDetection = useRef(false);
  const localConnectionRequest = useRef(0);

  useEffect(() => {
    if (!isTauri()) {
      setLoadingSettings(false);
      return undefined;
    }
    let cancelled = false;
    void (async () => {
      try {
        let snapshot = await invoke<AiServiceSettingsSnapshot>("get_ai_service_settings");
        if (!snapshot.agentCli && legacySettings) {
          await invoke<SavedAgentCliSettings>("save_agent_cli_settings", {
            settings: legacySettings,
          });
          snapshot = await invoke<AiServiceSettingsSnapshot>("get_ai_service_settings");
        }
        const agentSettings = snapshot.agentCli;
        if (agentSettings && isAgentCliKey(agentSettings.cli) && isAgentFilePermission(agentSettings.permission)) {
          localStorage.removeItem(agentCliSettingsStorageKey);
          if (!cancelled) {
            setSelectedAgent(agentSettings.cli);
            setPermission(agentSettings.permission);
            window.dispatchEvent(new CustomEvent("lumetrace:agent-cli-settings-changed", { detail: agentSettings }));
          }
        }
        if (snapshot.local && localProviders.includes(snapshot.local.provider) && !cancelled) {
          setLocalProvider(snapshot.local.provider);
          setLocalBaseUrl(snapshot.local.baseUrl);
          setLocalModels([snapshot.local.model]);
          setSelectedLocalModel(snapshot.local.model);
          setLocalConnectionState("idle");
        }
      } catch {
        // Keep a valid legacy value visible. A later explicit Save retries SQLite persistence.
      } finally {
        if (!cancelled) setLoadingSettings(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [legacySettings]);

  const cloudProviderOptions = useMemo<readonly MacSelectOption<CloudProvider>[]>(
    () => cloudProviders.map((value) => ({
      value,
      label: t(`fileSpace.settings.aiService.providers.${value}`),
    })),
    [t],
  );
  const localProviderOptions = useMemo<readonly MacSelectOption<LocalProvider>[]>(
    () => localProviders.map((value) => ({
      value,
      label: t(`fileSpace.settings.aiService.providers.${value}`),
    })),
    [t],
  );
  const localModelOptions = useMemo<readonly MacSelectOption<string>[]>(
    () => localModels.length > 0
      ? localModels.map((model) => ({ value: model, label: model }))
      : [{ value: "", label: t("fileSpace.settings.aiService.modelPlaceholder"), disabled: true }],
    [localModels, t],
  );
  const permissionOptions = useMemo<readonly MacSelectOption<AgentFilePermission>[]>(
    () => [
      { value: "readOnly", label: t("fileSpace.settings.aiService.readOnly") },
      { value: "readWrite", label: t("fileSpace.settings.aiService.readWrite") },
    ],
    [t],
  );

  const baseUrlPlaceholder = mode === "local"
    ? t("fileSpace.settings.aiService.baseUrlPlaceholder")
    : "https://api.openai.com/v1";

  const resetLocalConnection = () => {
    localConnectionRequest.current += 1;
    setLocalModels([]);
    setSelectedLocalModel("");
    setLocalConnectionState("idle");
    setSaveError(false);
  };

  const changeLocalProvider = (provider: LocalProvider) => {
    setLocalProvider(provider);
    setLocalBaseUrl(defaultLocalBaseUrl);
    resetLocalConnection();
  };

  const changeLocalBaseUrl = (baseUrl: string) => {
    setLocalBaseUrl(baseUrl);
    resetLocalConnection();
  };

  const detectAgentClis = useCallback(async () => {
    setDetectingAgents(true);
    setAgentChecks(createAgentCheckMap("checking"));
    const checks = await requestAgentChecks();
    cachedAgentChecks = checks;
    startupAgentCheckPromise = Promise.resolve(checks);
    setAgentChecks(checks);
    setDetectingAgents(false);
  }, []);

  useEffect(() => {
    if (mode !== "agentCli" || requestedStartupDetection.current) return;
    requestedStartupDetection.current = true;
    if (cachedAgentChecks) {
      setAgentChecks(reconcileRecentRuntimeSuccess(cachedAgentChecks));
      return;
    }
    setDetectingAgents(true);
    setAgentChecks(createAgentCheckMap("checking"));
    void getStartupAgentChecks().then((checks) => {
      setAgentChecks(checks);
      setDetectingAgents(false);
    });
  }, [mode]);

  const testSelectedAgent = async () => {
    const previousCheck = agentChecks[selectedAgent];
    if (!isTauri() || previousCheck.state === "checking") return;
    setAgentChecks((current) => ({
      ...current,
      [selectedAgent]: { ...current[selectedAgent], state: "checking" },
    }));
    try {
      const result = await invoke<AgentCliStatusResponse | null>("check_agent_cli", { key: selectedAgent });
      setAgentChecks((current) => {
        const next = {
          ...current,
          [selectedAgent]: result ? toAgentCheck(result) : { state: "missing" as const },
        };
        cachedAgentChecks = next;
        startupAgentCheckPromise = Promise.resolve(next);
        return next;
      });
    } catch {
      setAgentChecks((current) => {
        const next = {
          ...current,
          [selectedAgent]: { ...previousCheck, state: "error" as const },
        };
        cachedAgentChecks = next;
        startupAgentCheckPromise = Promise.resolve(next);
        return next;
      });
    }
  };

  const testLocalModelConnection = async () => {
    const baseUrl = localBaseUrl.trim();
    if (!baseUrl || !isTauri() || localConnectionState === "checking") return;
    const requestId = localConnectionRequest.current + 1;
    localConnectionRequest.current = requestId;
    setLocalConnectionState("checking");
    setSaveError(false);
    try {
      const result = await invoke<LocalLlmConnectionResult>("check_local_llm_connection", {
        request: { provider: localProvider, baseUrl },
      });
      if (localConnectionRequest.current !== requestId) return;
      setLocalBaseUrl(result.baseUrl);
      setLocalModels(result.models);
      setSelectedLocalModel((current) => result.models.includes(current) ? current : (result.models[0] ?? ""));
      setLocalConnectionState(result.models.length > 0 ? "passed" : "error");
    } catch {
      if (localConnectionRequest.current !== requestId) return;
      setLocalModels([]);
      setSelectedLocalModel("");
      setLocalConnectionState("error");
    }
  };

  const saveAgentCliSettings = async () => {
    const selectedCheck = agentChecks[selectedAgent];
    if (selectedCheck.state !== "passed" || !isTauri() || savingSettings) return;
    const settings: SavedAgentCliSettings = {
      cli: selectedAgent,
      permission,
      version: selectedCheck.version,
    };
    setSaveError(false);
    setSavingSettings(true);
    try {
      const saved = await invoke<SavedAgentCliSettings>("save_agent_cli_settings", { settings });
      localStorage.removeItem(agentCliSettingsStorageKey);
      window.dispatchEvent(new CustomEvent("lumetrace:agent-cli-settings-changed", { detail: saved }));
      window.dispatchEvent(new CustomEvent("lumetrace:ai-service-settings-changed", { detail: { mode: "agentCli" } }));
      onCancel();
    } catch {
      setSaveError(true);
    } finally {
      setSavingSettings(false);
    }
  };

  const saveLocalModelSettings = async () => {
    if (localConnectionState !== "passed" || !selectedLocalModel || !isTauri() || savingSettings) return;
    setSaveError(false);
    setSavingSettings(true);
    try {
      const saved = await invoke<LocalLlmSettings>("save_local_llm_settings", {
        settings: {
          provider: localProvider,
          baseUrl: localBaseUrl.trim(),
          model: selectedLocalModel,
        },
      });
      setLocalProvider(saved.provider);
      setLocalBaseUrl(saved.baseUrl);
      setSelectedLocalModel(saved.model);
      window.dispatchEvent(new CustomEvent("lumetrace:ai-service-settings-changed", { detail: { mode: "local", local: saved } }));
      onCancel();
    } catch {
      setSaveError(true);
    } finally {
      setSavingSettings(false);
    }
  };

  const selectedCheck = agentChecks[selectedAgent];
  const canTestSelectedAgent = selectedCheck.state !== "idle"
    && selectedCheck.state !== "checking"
    && selectedCheck.state !== "missing"
    && isTauri();
  const canSaveAgent = selectedCheck.state === "passed" && !loadingSettings && !savingSettings;
  const canTestLocal = Boolean(localBaseUrl.trim())
    && localConnectionState !== "checking"
    && !loadingSettings
    && isTauri();
  const canSaveLocal = localConnectionState === "passed"
    && Boolean(selectedLocalModel)
    && !loadingSettings
    && !savingSettings;
  const SelectedStatusIcon = selectedCheck.state === "passed"
    ? CircleCheck
    : selectedCheck.state === "checking"
      ? LoaderCircle
      : CircleAlert;
  const LocalStatusIcon = localConnectionState === "passed"
    ? CircleCheck
    : localConnectionState === "checking"
      ? LoaderCircle
      : localConnectionState === "error"
        ? CircleAlert
        : HardDrive;

  return (
    <>
      <div className="file-space-settings-ai-service">
        <p className="file-space-settings-ai-service-intro">
          {t("fileSpace.settings.aiService.description")}
        </p>

        <div
          className="file-space-ai-service-segment"
          role="group"
          aria-label={t("fileSpace.settings.aiService.modeLabel")}
        >
          <button className={mode === "local" ? "is-active" : ""} type="button" aria-pressed={mode === "local"} onClick={() => setMode("local")}>
            <HardDrive size={15} />
            {t("fileSpace.settings.aiService.local")}
          </button>
          <button className={mode === "cloud" ? "is-active" : ""} type="button" aria-pressed={mode === "cloud"} onClick={() => setMode("cloud")}>
            <Cloud size={15} />
            {t("fileSpace.settings.aiService.cloud")}
          </button>
          <button className={mode === "agentCli" ? "is-active" : ""} type="button" aria-pressed={mode === "agentCli"} onClick={() => setMode("agentCli")}>
            <Terminal size={15} />
            {t("fileSpace.settings.aiService.agentCli")}
          </button>
        </div>

        {mode === "agentCli" ? (
          <div className="file-space-ai-agent-settings">
            <div className="file-space-ai-agent-heading">
              <div>
                <strong>{t("fileSpace.settings.aiService.agentCliSectionTitle")}</strong>
                <span>{t("fileSpace.settings.aiService.agentCliSectionDescription")}</span>
              </div>
              <button type="button" disabled={detectingAgents} onClick={() => void detectAgentClis()}>
                <RefreshCw className={detectingAgents ? "is-spinning" : ""} size={14} />
                {t("fileSpace.settings.aiService.recheck")}
              </button>
            </div>

            <div className="file-space-ai-agent-grid" role="radiogroup" aria-label={t("fileSpace.settings.aiService.agentCliSectionTitle")}>
              {agentCliKeys.map((agent) => {
                const check = agentChecks[agent];
                return (
                  <button
                    className={`file-space-ai-agent-card is-${check.state}${selectedAgent === agent ? " is-selected" : ""}`}
                    type="button"
                    role="radio"
                    aria-checked={selectedAgent === agent}
                    disabled={check.state === "checking"}
                    key={agent}
                    onClick={() => setSelectedAgent(agent)}
                  >
                    <span className="file-space-ai-agent-icon"><AgentCliLogo agent={agent} /></span>
                    <span className="file-space-ai-agent-copy">
                      <strong>{t(`fileSpace.settings.aiService.providers.${agent}`)}</strong>
                      <small>{check.version ?? t(`fileSpace.settings.aiService.agentStatus.${check.state}`)}</small>
                    </span>
                    <span className={`file-space-ai-agent-badge is-${check.state}`}>
                      {t(`fileSpace.settings.aiService.agentStatus.${check.state}`)}
                    </span>
                  </button>
                );
              })}
            </div>

            <div className="file-space-ai-agent-permission">
              <span>{t("fileSpace.settings.aiService.permission")}</span>
              <MacSelect
                className="file-space-ai-service-select"
                value={permission}
                options={permissionOptions}
                onChange={setPermission}
                ariaLabel={t("fileSpace.settings.aiService.permission")}
                menuMinWidth={250}
              />
              <small>{t(permission === "readOnly"
                ? "fileSpace.settings.aiService.readOnlyDescription"
                : "fileSpace.settings.aiService.readWriteDescription")}</small>
            </div>

            <div className="file-space-ai-service-notice">
              <ShieldCheck size={18} aria-hidden="true" />
              <span>{t("fileSpace.settings.aiService.agentCliPrivacy")}</span>
            </div>

            <div className={`file-space-ai-agent-connection is-${selectedCheck.state}`} role="status" aria-live="polite">
              <SelectedStatusIcon className={selectedCheck.state === "checking" ? "is-spinning" : ""} size={18} />
              <div>
                <strong>{t(`fileSpace.settings.aiService.agentConnection.${selectedCheck.state}.title`)}</strong>
                <p>{t(`fileSpace.settings.aiService.agentConnection.${selectedCheck.state}.description`)}</p>
              </div>
            </div>
            {saveError ? (
              <div className="file-space-ai-agent-connection is-error" role="alert">
                <CircleAlert size={18} />
                <div><p>{t("fileSpace.settings.aiService.saveError")}</p></div>
              </div>
            ) : null}
          </div>
        ) : mode === "local" ? (
          <>
            <div className="file-space-ai-service-form">
              <span>{t("fileSpace.settings.aiService.provider")}</span>
              <MacSelect className="file-space-ai-service-select" value={localProvider} options={localProviderOptions} onChange={changeLocalProvider} ariaLabel={t("fileSpace.settings.aiService.provider")} menuMinWidth={250} />

              <label htmlFor="file-space-ai-service-base-url">{t("fileSpace.settings.aiService.baseUrl")}</label>
              <input
                id="file-space-ai-service-base-url"
                type="url"
                value={localBaseUrl}
                aria-describedby="file-space-ai-service-base-url-example"
                disabled={localConnectionState === "checking" || savingSettings}
                placeholder={baseUrlPlaceholder}
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                onChange={(event) => changeLocalBaseUrl(event.target.value)}
              />
              <small id="file-space-ai-service-base-url-example" className="file-space-ai-service-example">
                {t("fileSpace.settings.aiService.baseUrlExample", { url: localProviderExampleUrls[localProvider] })}
              </small>

              <span>{t("fileSpace.settings.aiService.model")}</span>
              <MacSelect
                className="file-space-ai-service-select"
                value={selectedLocalModel}
                options={localModelOptions}
                onChange={setSelectedLocalModel}
                ariaLabel={t("fileSpace.settings.aiService.model")}
                disabled={localConnectionState !== "passed" || savingSettings}
                menuMinWidth={250}
              />
            </div>

            <div className="file-space-ai-service-notice">
              <ShieldCheck size={18} aria-hidden="true" />
              <span>{t("fileSpace.settings.aiService.localPrivacy")}</span>
            </div>

            <div className={`file-space-ai-agent-connection is-${localConnectionState}`} role="status" aria-live="polite">
              <LocalStatusIcon className={localConnectionState === "checking" ? "is-spinning" : ""} size={18} />
              <div>
                <strong>{t(`fileSpace.settings.aiService.localConnection.${localConnectionState}.title`)}</strong>
                <p>{t(`fileSpace.settings.aiService.localConnection.${localConnectionState}.description`)}</p>
              </div>
            </div>
            {saveError ? (
              <div className="file-space-ai-agent-connection is-error" role="alert">
                <CircleAlert size={18} />
                <div><p>{t("fileSpace.settings.aiService.saveError")}</p></div>
              </div>
            ) : null}
          </>
        ) : (
          <>
            <div className="file-space-ai-service-form">
              <span>{t("fileSpace.settings.aiService.provider")}</span>
              <MacSelect className="file-space-ai-service-select" value={cloudProvider} options={cloudProviderOptions} onChange={setCloudProvider} ariaLabel={t("fileSpace.settings.aiService.provider")} menuMinWidth={250} />

              <label htmlFor="file-space-ai-service-base-url">{t("fileSpace.settings.aiService.baseUrl")}</label>
              <input id="file-space-ai-service-base-url" type="url" disabled placeholder={baseUrlPlaceholder} />

              <label htmlFor="file-space-ai-service-api-key">{t("fileSpace.settings.aiService.apiKey")}</label>
              <div className="file-space-ai-service-secret-field">
                <LockKeyhole size={14} aria-hidden="true" />
                <input id="file-space-ai-service-api-key" type="password" disabled placeholder={t("fileSpace.settings.aiService.apiKeyPlaceholder")} />
              </div>

              <span>{t("fileSpace.settings.aiService.model")}</span>
              <MacSelect className="file-space-ai-service-select" value="" options={[{ value: "", label: t("fileSpace.settings.aiService.cloudModelPlaceholder"), disabled: true }]} onChange={() => undefined} ariaLabel={t("fileSpace.settings.aiService.model")} disabled menuMinWidth={250} />
            </div>

            <div className="file-space-ai-service-notice">
              <ShieldCheck size={18} aria-hidden="true" />
              <span>{t("fileSpace.settings.aiService.cloudPrivacy")}</span>
            </div>

            <div className="file-space-ai-service-status" role="status">
              <span aria-hidden="true" />
              <div>
                <strong>{t("fileSpace.settings.aiService.uiOnlyTitle")}</strong>
                <p>{t("fileSpace.settings.aiService.uiOnlyDescription")}</p>
              </div>
            </div>
          </>
        )}
      </div>

      <footer>
        <button type="button" onClick={onCancel}>{t("fileSpace.settings.cancel")}</button>
        <button
          type="button"
          disabled={mode === "local" ? !canTestLocal : mode === "agentCli" ? !canTestSelectedAgent : true}
          onClick={() => void (mode === "local" ? testLocalModelConnection() : testSelectedAgent())}
        >
          {mode === "local" && localConnectionState === "checking"
            ? t("fileSpace.settings.aiService.testingConnection")
            : mode === "agentCli" && selectedCheck.state === "checking"
            ? t("fileSpace.settings.aiService.testingConnection")
            : t("fileSpace.settings.aiService.testConnection")}
        </button>
        <button
          className="is-primary"
          type="button"
          disabled={mode === "local" ? !canSaveLocal : mode === "agentCli" ? !canSaveAgent : true}
          onClick={() => void (mode === "local" ? saveLocalModelSettings() : saveAgentCliSettings())}
        >
          {t("fileSpace.settings.aiService.save")}
        </button>
      </footer>
    </>
  );
}
