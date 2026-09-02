import { useEffect, useMemo, useState, type PropsWithChildren } from "react";
import {
  ThemeContext,
  type AppearancePreferences,
  type Theme,
  type ThemePreference,
  type UiFont,
} from "./theme-context";

const themeStorageKey = "lumetrace.theme.preference";
const legacyThemeStorageKey = "lumetrace.theme";
const appearanceStorageKey = "lumetrace.appearance";
const hexColorPattern = /^#[0-9a-f]{6}$/i;

export const defaultAppearance: AppearancePreferences = {
  highlightColor: "#0A84FF",
  uiFont: "system",
  bodyFontSize: 13,
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
    const isLegacyUntouchedDefault = parsed.highlightColor?.toUpperCase() === "#3F746A"
      && parsed.uiFont === "system"
      && Number(parsed.bodyFontSize) === 14
      && Number(parsed.codeFontSize) === 13
      && parsed.fontSmoothing === true;
    if (isLegacyUntouchedDefault) return defaultAppearance;
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

function getInitialThemePreference(): ThemePreference {
  const savedTheme = localStorage.getItem(themeStorageKey);
  if (savedTheme === "dark" || savedTheme === "light" || savedTheme === "system") {
    return savedTheme;
  }
  const legacyTheme = localStorage.getItem(legacyThemeStorageKey);
  if (legacyTheme === "dark" || legacyTheme === "light") return legacyTheme;
  return "system";
}

export function ThemeProvider({ children }: PropsWithChildren) {
  const systemThemeQuery = useMemo(() => window.matchMedia("(prefers-color-scheme: dark)"), []);
  const previewTheme = useMemo<Theme | null>(() => {
    if (!import.meta.env.DEV) return null;
    const value = new URLSearchParams(window.location.search).get("theme");
    return value === "light" || value === "dark" ? value : null;
  }, []);
  const [systemTheme, setSystemTheme] = useState<Theme>(() => systemThemeQuery.matches ? "dark" : "light");
  const [themePreference, setTheme] = useState<ThemePreference>(getInitialThemePreference);
  const [appearance, setAppearance] = useState<AppearancePreferences>(getInitialAppearance);
  const theme = previewTheme ?? (themePreference === "system" ? systemTheme : themePreference);

  useEffect(() => {
    const updateSystemTheme = (event: MediaQueryListEvent) => setSystemTheme(event.matches ? "dark" : "light");
    systemThemeQuery.addEventListener("change", updateSystemTheme);
    return () => systemThemeQuery.removeEventListener("change", updateSystemTheme);
  }, [systemThemeQuery]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.themePreference = themePreference;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem(themeStorageKey, themePreference);
    localStorage.removeItem(legacyThemeStorageKey);
  }, [theme, themePreference]);

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
      themePreference,
      setTheme,
      toggleTheme: () => setTheme(theme === "dark" ? "light" : "dark"),
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
    [appearance, theme, themePreference],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
