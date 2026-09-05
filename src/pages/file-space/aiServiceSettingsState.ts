export type AiServiceMode = "cloud" | "local" | "agentCli";

// Opening Settings is a navigation choice, not a reflection of the currently
// active backend. Saved services remain configured without taking over this tab.
export const defaultAiServiceMode: AiServiceMode = "local";

// A service address is deployment-specific. Keep a new configuration empty
// and present provider defaults as examples instead of assumed values.
export const defaultLocalBaseUrl = "";

export function retainedCloudApiKeyVisibility(
  current: boolean,
  mode: AiServiceMode,
  visible: boolean,
) {
  return current && mode === "cloud" && visible;
}

interface AiServiceConfigurationSnapshot {
  mode: AiServiceMode | null;
  cloud?: {
    model?: string;
    hasApiKey?: boolean;
  } | null;
  local?: unknown | null;
  agentCli?: {
    cli?: string;
    permission?: string;
  } | null;
}

export const questionAnswerAgentCliKeys = ["claude", "hermes", "codex", "opencode"] as const;

export function isAiServiceConfigured(settings: AiServiceConfigurationSnapshot) {
  if (settings.mode === "cloud") {
    return Boolean(settings.cloud?.hasApiKey && settings.cloud.model?.trim());
  }
  if (settings.mode === "local") return Boolean(settings.local);
  if (settings.mode === "agentCli") return Boolean(settings.agentCli);
  return false;
}

export function canAskConfiguredAiService(settings: AiServiceConfigurationSnapshot) {
  if (settings.mode === "cloud") {
    return Boolean(settings.cloud?.hasApiKey && settings.cloud.model?.trim());
  }
  if (settings.mode === "local") return Boolean(settings.local);
  if (settings.mode !== "agentCli") return false;
  return settings.agentCli?.permission === "readOnly"
    && questionAnswerAgentCliKeys.includes(
      settings.agentCli.cli as (typeof questionAnswerAgentCliKeys)[number],
    );
}
