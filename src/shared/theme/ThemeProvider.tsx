import { useEffect, useLayoutEffect, useMemo, useState, type PropsWithChildren } from "react";
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
  system: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Segoe UI", "Microsoft YaHei", sans-serif',
  inter: 'Inter, -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Segoe UI", "Microsoft YaHei", sans-serif',
  pingfang: '"PingFang SC", -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", sans-serif',
  "microsoft-yahei": '"Microsoft YaHei", "Segoe UI", system-ui, sans-serif',
};

const uiFonts = new Set<UiFont>(["system", "inter", "pingfang", "microsoft-yahei"]);

function clampSize(value: unknown, minimum: number, maximum: number, fallback: number) {
  const numericValue = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numericValue)) return fallback;
  return Math.min(maximum, Math.max(minimum, Math.round(numericValue)));
}

interface RgbColor {
  red: number;
  green: number;
  blue: number;
}

function parseHexColor(hexColor: string): RgbColor {
  return {
    red: Number.parseInt(hexColor.slice(1, 3), 16),
    green: Number.parseInt(hexColor.slice(3, 5), 16),
    blue: Number.parseInt(hexColor.slice(5, 7), 16),
  };
}

function hexColor({ red, green, blue }: RgbColor) {
  return `#${[red, green, blue]
    .map((channel) => Math.round(channel).toString(16).padStart(2, "0"))
    .join("")}`.toUpperCase();
}

function relativeLuminance(color: RgbColor) {
  const [red, green, blue] = [color.red, color.green, color.blue].map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return red * 0.2126 + green * 0.7152 + blue * 0.0722;
}

function contrastRatio(first: RgbColor, second: RgbColor) {
  const light = Math.max(relativeLuminance(first), relativeLuminance(second));
  const dark = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (light + 0.05) / (dark + 0.05);
}

function blendColor(source: RgbColor, target: RgbColor, amount: number): RgbColor {
  return {
    red: source.red + (target.red - source.red) * amount,
    green: source.green + (target.green - source.green) * amount,
    blue: source.blue + (target.blue - source.blue) * amount,
  };
}

function getAdaptiveHighlight(hexValue: string, theme: Theme) {
  const source = parseHexColor(hexValue);
  const surface = parseHexColor(theme === "dark" ? "#2C2C2E" : "#FFFFFF");
  if (contrastRatio(source, surface) >= 3) return hexValue.toUpperCase();
  const contrastTarget = parseHexColor(theme === "dark" ? "#FFFFFF" : "#000000");
  for (let step = 1; step <= 20; step += 1) {
    const adjusted = blendColor(source, contrastTarget, step / 20);
    if (contrastRatio(adjusted, surface) >= 3) return hexColor(adjusted);
  }
  return hexColor(contrastTarget);
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

  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.themePreference = themePreference;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem(themeStorageKey, themePreference);
    localStorage.removeItem(legacyThemeStorageKey);
  }, [theme, themePreference]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    const highlightColor = getAdaptiveHighlight(appearance.highlightColor, theme);
    root.style.setProperty("--highlight", highlightColor);
    root.style.removeProperty("--highlight-foreground");
    root.style.setProperty("--font-family-ui", uiFontStacks[appearance.uiFont]);
    root.style.setProperty("--font-size-body", `${appearance.bodyFontSize}px`);
    root.style.setProperty("--font-size-code", `${appearance.codeFontSize}px`);
    root.dataset.fontSmoothing = appearance.fontSmoothing ? "on" : "off";
    localStorage.setItem(appearanceStorageKey, JSON.stringify(appearance));
  }, [appearance, theme]);

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
