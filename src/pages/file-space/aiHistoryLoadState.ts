export type AiConfigurationState = "loading" | "configured" | "unconfigured" | "error";
export type AiHistoryState = "idle" | "loading" | "ready" | "error";

export interface AiSurfaceLoadState<TSettings> {
  configurationState: AiConfigurationState;
  serviceSettings: TSettings | null;
  historyState: AiHistoryState;
}

export function initialAiSurfaceLoadState<TSettings>(): AiSurfaceLoadState<TSettings> {
  return {
    configurationState: "loading",
    serviceSettings: null,
    historyState: "idle",
  };
}

export function aiConfigurationLoadStarted<TSettings>(
  current: AiSurfaceLoadState<TSettings>,
): AiSurfaceLoadState<TSettings> {
  return {
    ...current,
    configurationState: "loading",
  };
}

export function aiConfigurationLoadResolved<TSettings>(
  settings: TSettings,
  configured: boolean,
): AiSurfaceLoadState<TSettings> {
  return {
    configurationState: configured ? "configured" : "unconfigured",
    serviceSettings: settings,
    historyState: configured ? "loading" : "idle",
  };
}

export function aiConfigurationLoadFailed<TSettings>(): AiSurfaceLoadState<TSettings> {
  return {
    configurationState: "error",
    serviceSettings: null,
    historyState: "idle",
  };
}

export function aiHistoryLoadStarted<TSettings>(
  current: AiSurfaceLoadState<TSettings>,
): AiSurfaceLoadState<TSettings> {
  return {
    ...current,
    historyState: "loading",
  };
}

export function aiHistoryLoadSucceeded<TSettings>(
  current: AiSurfaceLoadState<TSettings>,
): AiSurfaceLoadState<TSettings> {
  return {
    ...current,
    historyState: "ready",
  };
}

export function aiHistoryLoadFailed<TSettings>(
  current: AiSurfaceLoadState<TSettings>,
): AiSurfaceLoadState<TSettings> {
  return {
    ...current,
    historyState: "error",
  };
}
