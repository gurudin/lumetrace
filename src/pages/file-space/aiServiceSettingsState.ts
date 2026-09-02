export type AiServiceMode = "cloud" | "local" | "agentCli";

// Opening Settings is a navigation choice, not a reflection of the currently
// active backend. Saved services remain configured without taking over this tab.
export const defaultAiServiceMode: AiServiceMode = "local";
