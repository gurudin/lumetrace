import { useEffect, useMemo, useState, type PropsWithChildren } from "react";
import {
  ThemeContext,
  type AppearancePreferences,
  type Theme,
  type UiFont,
} from "./theme-context";

const themeStorageKey = "lumetrace.theme";
const appearanceStorageKey = "lumetrace.appearance";
const hexColorPattern = /^#[0-9a-f]{6}$/i;

export const defaultAppearance: AppearancePreferences = {
  highlightColor: "#3F746A",
  uiFont: "system",
  bodyFontSize: 14,
  codeFontSize: 13,
  fontSmoothing: true,
};

const uiFontStacks: Record<UiFont, string> = {
  system: 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
  inter: 'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
  pingfang: '"PingFang SC", -apple-system, BlinkMacSystemFont, system-ui, "Segoe UI", sans-serif',
  "microsoft-yahei": '"Microsoft YaHei", "Segoe UI", system-ui, sans-serif',
};

const uiFonts = new Set<UiFont>(["system", "inter", "pingfang", "microsoft-yahei"]);

function clampSize(value: unknown, minimum: number, maximum: number, fallback: number) {
  const numericValue = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numericValue)) return fallback;
  return Math.min(maximum, Math.max(minimum, Math.round(numericValue)));
}

function getContrastColor(hexColor: string) {
  const red = Number.parseInt(hexColor.slice(1, 3), 16);
  const green = Number.parseInt(hexColor.slice(3, 5), 16);
  const blue = Number.parseInt(hexColor.slice(5, 7), 16);
  const perceivedBrightness = (red * 299 + green * 587 + blue * 114) / 1000;
  return perceivedBrightness > 164 ? "#111214" : "#FFFFFF";
}

function getInitialAppearance(): AppearancePreferences {
  const savedAppearance = localStorage.getItem(appearanceStorageKey);
  if (!savedAppearance) return defaultAppearance;

  try {
    const parsed = JSON.parse(savedAppearance) as Partial<AppearancePreferences>;
    return {
      highlightColor: typeof parsed.highlightColor === "string" && hexColorPattern.test(parsed.highlightColor)
        ? parsed.highlightColor.toUpperCase()
        : defaultAppearance.highlightColor,
      uiFont: typeof parsed.uiFont === "string" && uiFonts.has(parsed.uiFont as UiFont)
        ? parsed.uiFont as UiFont
        : defaultAppearance.uiFont,
      bodyFontSize: clampSize(parsed.bodyFontSize, 12, 18, defaultAppearance.bodyFontSize),
      codeFontSize: clampSize(parsed.codeFontSize, 11, 18, defaultAppearance.codeFontSize),
      fontSmoothing: typeof parsed.fontSmoothing === "boolean"
        ? parsed.fontSmoothing
        : defaultAppearance.fontSmoothing,
    };
  } catch {
    return defaultAppearance;
  }
}

function getInitialTheme(): Theme {
  const savedTheme = localStorage.getItem(themeStorageKey);
  if (savedTheme === "dark" || savedTheme === "light") {
    return savedTheme;
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function ThemeProvider({ children }: PropsWithChildren) {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [appearance, setAppearance] = useState<AppearancePreferences>(getInitialAppearance);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem(themeStorageKey, theme);
  }, [theme]);

  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--highlight", appearance.highlightColor);
    root.style.setProperty("--highlight-foreground", getContrastColor(appearance.highlightColor));
    root.style.setProperty("--font-family-ui", uiFontStacks[appearance.uiFont]);
    root.style.setProperty("--font-size-body", `${appearance.bodyFontSize}px`);
    root.style.setProperty("--font-size-code", `${appearance.codeFontSize}px`);
    root.dataset.fontSmoothing = appearance.fontSmoothing ? "on" : "off";
    localStorage.setItem(appearanceStorageKey, JSON.stringify(appearance));
  }, [appearance]);

  const value = useMemo(
    () => ({
      theme,
      setTheme,
      toggleTheme: () => setTheme((current) => (current === "dark" ? "light" : "dark")),
      appearance,
      setHighlightColor: (highlightColor: string) => {
        if (!hexColorPattern.test(highlightColor)) return;
        setAppearance((current) => ({ ...current, highlightColor: highlightColor.toUpperCase() }));
      },
      setUiFont: (uiFont: UiFont) => setAppearance((current) => ({ ...current, uiFont })),
      setBodyFontSize: (bodyFontSize: number) => setAppearance((current) => ({
        ...current,
        bodyFontSize: clampSize(bodyFontSize, 12, 18, current.bodyFontSize),
      })),
      setCodeFontSize: (codeFontSize: number) => setAppearance((current) => ({
        ...current,
        codeFontSize: clampSize(codeFontSize, 11, 18, current.codeFontSize),
      })),
      setFontSmoothing: (fontSmoothing: boolean) => setAppearance((current) => ({
        ...current,
        fontSmoothing,
      })),
      resetAppearance: () => setAppearance(defaultAppearance),
    }),
    [appearance, theme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
