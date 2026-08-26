import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { en } from "./locales/en";
import { zh } from "./locales/zh";

const languageStorageKey = "lumetrace.language";
const savedLanguage = localStorage.getItem(languageStorageKey);
const browserLanguage = navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
const initialLanguage = savedLanguage?.startsWith("zh") ? "zh" : savedLanguage === "en" ? "en" : browserLanguage;

document.documentElement.lang = initialLanguage === "zh" ? "zh-CN" : "en";

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
  },
  lng: initialLanguage,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

i18n.on("languageChanged", (language) => {
  const normalizedLanguage = language.startsWith("zh") ? "zh" : "en";
  localStorage.setItem(languageStorageKey, normalizedLanguage);
  document.documentElement.lang = normalizedLanguage === "zh" ? "zh-CN" : "en";
});

export default i18n;
