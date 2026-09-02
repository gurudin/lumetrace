import { createContext } from "react";

export type Theme = "dark" | "light";
export type ThemePreference = Theme | "system";
export type UiFont = "system" | "inter" | "pingfang" | "microsoft-yahei";

export interface AppearancePreferences {
  highlightColor: string;
  uiFont: UiFont;
  bodyFontSize: number;
  codeFontSize: number;
  fontSmoothing: boolean;
}

export interface ThemeContextValue {
  theme: Theme;
  themePreference: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  toggleTheme: () => void;
  appearance: AppearancePreferences;
  setHighlightColor: (color: string) => void;
  setUiFont: (font: UiFont) => void;
  setBodyFontSize: (size: number) => void;
  setCodeFontSize: (size: number) => void;
  setFontSmoothing: (enabled: boolean) => void;
  resetAppearance: () => void;
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);
