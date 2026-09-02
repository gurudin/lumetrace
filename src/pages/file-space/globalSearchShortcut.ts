export interface ShortcutEventLike {
  key: string;
  metaKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
}

function normalizedPlatform(platform?: string) {
  if (platform) return platform;
  if (typeof navigator === "undefined") return "";
  const userAgentPlatform = (navigator as Navigator & {
    userAgentData?: { platform?: string };
  }).userAgentData?.platform;
  return userAgentPlatform || navigator.platform || navigator.userAgent;
}

export function usesMacSearchShortcut(platform?: string) {
  return /Mac|iPhone|iPad|iPod/i.test(normalizedPlatform(platform));
}

export function globalSearchShortcutLabel(platform?: string) {
  return usesMacSearchShortcut(platform) ? "⌘K" : "Alt K";
}

export function isGlobalSearchShortcut(event: ShortcutEventLike, platform?: string) {
  if (event.isComposing || event.key.toLocaleLowerCase() !== "k" || event.ctrlKey || event.shiftKey) {
    return false;
  }
  return usesMacSearchShortcut(platform)
    ? event.metaKey && !event.altKey
    : event.altKey && !event.metaKey;
}
