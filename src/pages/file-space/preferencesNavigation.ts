export const preferencesSectionKeys = [
  "appearance",
  "general",
  "aiService",
  "background",
] as const;

export type PreferencesSection = typeof preferencesSectionKeys[number];

export const openAiServiceSettingsEventName = "lumetrace:open-ai-service-settings";
export const openBackgroundStatusEventName = "lumetrace:open-background-status";

export function preferencesSectionForOpenEvent(eventName: string): PreferencesSection | null {
  if (eventName === openAiServiceSettingsEventName) return "aiService";
  if (eventName === openBackgroundStatusEventName) return "background";
  return null;
}
