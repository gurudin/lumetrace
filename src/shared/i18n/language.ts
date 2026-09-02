export type SupportedLanguage =
  | "en-US"
  | "zh-CN"
  | "zh-TW"
  | "de-DE"
  | "ja-JP"
  | "ko-KR"
  | "fr-FR"
  | "es-ES";

export type LanguagePreference = SupportedLanguage | "system";

export interface LanguageOption {
  code: SupportedLanguage;
  nativeName: string;
}

export const languagePreferenceStorageKey = "lumetrace.language.preference";
export const legacyLanguageStorageKey = "lumetrace.language";

export const supportedLanguages: readonly LanguageOption[] = [
  { code: "en-US", nativeName: "English" },
  { code: "zh-CN", nativeName: "简体中文" },
  { code: "zh-TW", nativeName: "繁體中文" },
  { code: "de-DE", nativeName: "Deutsch" },
  { code: "ja-JP", nativeName: "日本語" },
  { code: "ko-KR", nativeName: "한국어" },
  { code: "fr-FR", nativeName: "Français" },
  { code: "es-ES", nativeName: "Español" },
];

export function matchSupportedLanguage(language: string | null | undefined): SupportedLanguage | null {
  if (!language) return null;
  const normalized = language.trim().replaceAll("_", "-").toLowerCase();
  if (!normalized) return null;
  if (normalized === "zh-tw" || normalized === "zh-hk" || normalized === "zh-mo" || normalized.startsWith("zh-hant")) {
    return "zh-TW";
  }
  if (normalized === "zh" || normalized === "zh-cn" || normalized === "zh-sg" || normalized.startsWith("zh-hans")) {
    return "zh-CN";
  }
  if (normalized === "en" || normalized.startsWith("en-")) return "en-US";
  if (normalized === "de" || normalized.startsWith("de-")) return "de-DE";
  if (normalized === "ja" || normalized.startsWith("ja-")) return "ja-JP";
  if (normalized === "ko" || normalized.startsWith("ko-")) return "ko-KR";
  if (normalized === "fr" || normalized.startsWith("fr-")) return "fr-FR";
  if (normalized === "es" || normalized.startsWith("es-")) return "es-ES";
  return null;
}

export function getSystemLanguage(languageCandidates?: readonly string[]): SupportedLanguage {
  const candidates = languageCandidates
    ?? (typeof navigator === "undefined" ? [] : navigator.languages);
  for (const candidate of candidates) {
    const supported = matchSupportedLanguage(candidate);
    if (supported) return supported;
  }
  return "en-US";
}

export function resolveLanguagePreference(
  preference: LanguagePreference,
  languageCandidates?: readonly string[],
): SupportedLanguage {
  return preference === "system" ? getSystemLanguage(languageCandidates) : preference;
}
