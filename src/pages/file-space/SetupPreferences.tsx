import { Globe2, SunMoon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLanguagePreference,
  setLanguagePreference,
  supportedLanguages,
  type LanguagePreference,
} from "../../shared/i18n/i18n";
import { getSystemLanguage } from "../../shared/i18n/language";
import type { ThemePreference } from "../../shared/theme/theme-context";
import { useTheme } from "../../shared/theme/useTheme";
import { MacSelect, type MacSelectOption } from "../../shared/ui/MacSelect";

const languageNames = new Map(supportedLanguages.map(({ code, nativeName }) => [code, nativeName]));

function isThemePreference(value: string): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function SetupPreferences() {
  const { t } = useTranslation();
  const { themePreference, setTheme } = useTheme();
  const [languagePreference, setLanguageChoice] = useState<LanguagePreference>(getLanguagePreference);
  const systemLanguage = getSystemLanguage();
  const systemLanguageName = languageNames.get(systemLanguage) ?? "English";
  const languageOptions: readonly MacSelectOption<LanguagePreference>[] = [
    {
      value: "system",
      label: t("fileSpace.preferences.language.system", { language: systemLanguageName }),
    },
    ...supportedLanguages.map(({ code, nativeName }) => ({ value: code, label: nativeName })),
  ];
  const appearanceOptions: readonly MacSelectOption<ThemePreference>[] = [
    { value: "system", label: t("fileSpace.preferences.appearance.system") },
    { value: "light", label: t("fileSpace.preferences.appearance.light") },
    { value: "dark", label: t("fileSpace.preferences.appearance.dark") },
  ];

  return (
    <aside className="file-space-setup-preferences" aria-label={t("fileSpace.preferences.ariaLabel")}>
      <MacSelect
        className="file-space-setup-language-select"
        value={languagePreference}
        options={languageOptions}
        ariaLabel={t("fileSpace.preferences.language.label")}
        icon={<Globe2 size={15} />}
        menuMinWidth={222}
        onChange={(preference) => {
          setLanguageChoice(preference);
          void setLanguagePreference(preference);
        }}
      />

      <MacSelect
        className="file-space-setup-appearance-select"
        value={themePreference}
        options={appearanceOptions}
        ariaLabel={t("fileSpace.preferences.appearance.label")}
        icon={<SunMoon size={15} />}
        menuAlign="end"
        menuMinWidth={156}
        onChange={(preference) => {
          if (isThemePreference(preference)) setTheme(preference);
        }}
      />
    </aside>
  );
}
