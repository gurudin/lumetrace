import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { de } from "./locales/de";
import { en } from "./locales/en";
import { es } from "./locales/es";
import { fr } from "./locales/fr";
import { ja } from "./locales/ja";
import { ko } from "./locales/ko";
import { zh } from "./locales/zh";
import { zhTW } from "./locales/zhTW";
import {
  getSystemLanguage,
  languagePreferenceStorageKey,
  legacyLanguageStorageKey,
  matchSupportedLanguage,
  resolveLanguagePreference,
  supportedLanguages,
  type LanguagePreference,
  type SupportedLanguage,
} from "./language";

export { supportedLanguages } from "./language";
export type { LanguagePreference, SupportedLanguage } from "./language";

function readLanguagePreference(): LanguagePreference {
  const savedPreference = localStorage.getItem(languagePreferenceStorageKey);
  if (savedPreference === "system") return "system";
  const savedLanguage = matchSupportedLanguage(savedPreference);
  if (savedLanguage) return savedLanguage;

  const legacyLanguage = matchSupportedLanguage(localStorage.getItem(legacyLanguageStorageKey));
  if (legacyLanguage) {
    localStorage.setItem(languagePreferenceStorageKey, legacyLanguage);
    localStorage.removeItem(legacyLanguageStorageKey);
    return legacyLanguage;
  }
  return "system";
}

export function getLanguagePreference(): LanguagePreference {
  return readLanguagePreference();
}

export function getResolvedLanguage(): SupportedLanguage {
  return matchSupportedLanguage(i18n.resolvedLanguage) ?? getSystemLanguage();
}

export async function setLanguagePreference(preference: LanguagePreference) {
  localStorage.setItem(languagePreferenceStorageKey, preference);
  localStorage.removeItem(legacyLanguageStorageKey);
  await i18n.changeLanguage(resolveLanguagePreference(preference));
}

const initialPreference = readLanguagePreference();
const initialLanguage = resolveLanguagePreference(initialPreference);

document.documentElement.lang = initialLanguage;

void i18n.use(initReactI18next).init({
  resources: {
    "en-US": { translation: en },
    "zh-CN": { translation: zh },
    "zh-TW": { translation: zhTW },
    "de-DE": { translation: de },
    "ja-JP": { translation: ja },
    "ko-KR": { translation: ko },
    "fr-FR": { translation: fr },
    "es-ES": { translation: es },
  },
  supportedLngs: supportedLanguages.map(({ code }) => code),
  load: "currentOnly",
  lng: initialLanguage,
  fallbackLng: "en-US",
  interpolation: { escapeValue: false },
});

i18n.on("languageChanged", (language) => {
  document.documentElement.lang = matchSupportedLanguage(language) ?? "en-US";
});

window.addEventListener("languagechange", () => {
  if (readLanguagePreference() !== "system") return;
  void i18n.changeLanguage(getSystemLanguage());
});

export default i18n;
