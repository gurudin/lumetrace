import {
  Activity,
  Bot,
  Check,
  Languages,
  Monitor,
  Moon,
  Palette,
  RotateCcw,
  Settings2,
  Sun,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLanguagePreference,
  setLanguagePreference,
  supportedLanguages,
  type LanguagePreference,
} from "../../shared/i18n/i18n";
import { getSystemLanguage } from "../../shared/i18n/language";
import {
  getHelpButtonVisibility,
  helpButtonVisibilityChangeEvent,
  setHelpButtonVisibility,
} from "../../shared/help/helpButtonVisibility";
import type { ThemePreference, UiFont } from "../../shared/theme/theme-context";
import { useTheme } from "../../shared/theme/useTheme";
import { MacSelect, type MacSelectOption } from "../../shared/ui/MacSelect";
import { AiServiceSettings } from "./AiServiceSettings";
import { FileSpaceBackgroundTasks } from "./FileSpaceBackgroundTasks";
import {
  preferencesSectionKeys,
  type PreferencesSection,
} from "./preferencesNavigation";

interface FileSpacePreferencesProps {
  initialSection?: PreferencesSection;
  navigationRequest?: number;
  onClose: () => void;
  onOpenSemantic: () => void;
}

const languageNames = new Map(supportedLanguages.map(({ code, nativeName }) => [code, nativeName]));
const bodyFontSizes = [12, 13, 14, 15, 16, 17, 18] as const;
const codeFontSizes = [11, 12, 13, 14, 15, 16, 17, 18] as const;
const optionalUiFonts: readonly { value: Exclude<UiFont, "system">; label: string; family: string }[] = [
  { value: "inter", label: "Inter", family: "Inter" },
  { value: "pingfang", label: "PingFang SC", family: "PingFang SC" },
  { value: "microsoft-yahei", label: "Microsoft YaHei", family: "Microsoft YaHei" },
];
const fontAvailability = new Map<string, boolean>();

function isFontAvailable(fontFamily: string) {
  const cached = fontAvailability.get(fontFamily);
  if (cached !== undefined) return cached;
  const context = document.createElement("canvas").getContext("2d");
  if (!context) return false;
  const sample = "mmmmmmWWWW你我测试漢字あア한글";
  const fallbacks = ["monospace", "serif", "sans-serif"];
  const available = fallbacks.some((fallback) => {
    context.font = `72px ${fallback}`;
    const fallbackWidth = context.measureText(sample).width;
    context.font = `72px "${fontFamily}", ${fallback}`;
    return Math.abs(context.measureText(sample).width - fallbackWidth) > 0.1;
  });
  fontAvailability.set(fontFamily, available);
  return available;
}

function sizeOptions(values: readonly number[]) {
  return values.map((value) => ({ value: String(value), label: `${value} px` }));
}

export function FileSpacePreferences({
  initialSection = "general",
  navigationRequest = 0,
  onClose,
  onOpenSemantic,
}: FileSpacePreferencesProps) {
  const { t } = useTranslation();
  const {
    themePreference,
    setTheme,
    appearance,
    setHighlightColor,
    setUiFont,
    setBodyFontSize,
    setCodeFontSize,
    setFontSmoothing,
    resetAppearance,
  } = useTheme();
  const [activeSection, setActiveSection] = useState<PreferencesSection>(initialSection);
  const [hasMountedAiService, setHasMountedAiService] = useState(initialSection === "aiService");
  const [languagePreference, setLanguageChoice] = useState<LanguagePreference>(getLanguagePreference);
  const [helpButtonVisible, setHelpButtonVisibleState] = useState(getHelpButtonVisibility);
  const contentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setActiveSection(initialSection);
    if (initialSection === "aiService") setHasMountedAiService(true);
  }, [initialSection, navigationRequest]);

  useLayoutEffect(() => {
    if (contentRef.current) contentRef.current.scrollTop = 0;
  }, [activeSection, navigationRequest]);

  useEffect(() => {
    const updateVisibility = () => setHelpButtonVisibleState(getHelpButtonVisibility());
    window.addEventListener(helpButtonVisibilityChangeEvent, updateVisibility);
    window.addEventListener("storage", updateVisibility);
    return () => {
      window.removeEventListener(helpButtonVisibilityChangeEvent, updateVisibility);
      window.removeEventListener("storage", updateVisibility);
    };
  }, []);

  const systemLanguage = getSystemLanguage();
  const systemLanguageName = languageNames.get(systemLanguage) ?? "English";
  const languageOptions = useMemo<readonly MacSelectOption<LanguagePreference>[]>(() => [
    {
      value: "system",
      label: t("fileSpace.preferences.language.system", { language: systemLanguageName }),
    },
    ...supportedLanguages.map(({ code, nativeName }) => ({ value: code, label: nativeName })),
  ], [systemLanguageName, t]);
  const uiFontOptions = useMemo<readonly MacSelectOption<UiFont>[]>(() => [
    { value: "system", label: t("fileSpace.preferences.center.systemDefault") },
    ...optionalUiFonts
      .filter(({ family }) => isFontAvailable(family))
      .map(({ value, label }) => ({ value, label })),
  ], [t]);
  const bodySizeOptions = useMemo(() => sizeOptions(bodyFontSizes), []);
  const codeSizeOptions = useMemo(() => sizeOptions(codeFontSizes), []);

  const sectionIcons: Record<PreferencesSection, typeof Languages> = {
    general: Settings2,
    appearance: Palette,
    aiService: Bot,
    background: Activity,
  };
  const sectionLabels: Record<PreferencesSection, string> = {
    general: t("fileSpace.preferences.center.sections.general"),
    appearance: t("fileSpace.preferences.center.sections.appearance"),
    aiService: t("fileSpace.settings.aiServiceTitle"),
    background: t("fileSpace.settings.backgroundTitle"),
  };
  const themes: readonly { key: ThemePreference; icon: typeof Monitor }[] = [
    { key: "system", icon: Monitor },
    { key: "light", icon: Sun },
    { key: "dark", icon: Moon },
  ];

  return (
    <div className="file-space-preferences">
      <nav className="file-space-preferences-sidebar" aria-label={t("fileSpace.settings.preferences")}>
        {preferencesSectionKeys.map((key) => {
          const Icon = sectionIcons[key];
          return (
          <button
            className={activeSection === key ? "is-active" : ""}
            type="button"
            aria-current={activeSection === key ? "page" : undefined}
            key={key}
            onClick={() => {
              setActiveSection(key);
              if (key === "aiService") setHasMountedAiService(true);
            }}
          >
            <Icon size={16} aria-hidden="true" />
            <span>{sectionLabels[key]}</span>
          </button>
          );
        })}
      </nav>

      <div ref={contentRef} className="file-space-preferences-content">
        {activeSection === "general" ? (
          <section className="file-space-preferences-page" aria-labelledby="file-space-preferences-general-title">
            <header>
              <h3 id="file-space-preferences-general-title">{t("fileSpace.preferences.center.sections.general")}</h3>
              <p>{t("fileSpace.preferences.center.generalDescription")}</p>
            </header>
            <div className="file-space-preferences-group">
              <div className="file-space-preferences-row">
                <div>
                  <strong>{t("fileSpace.preferences.language.label")}</strong>
                  <span>{t("fileSpace.preferences.center.languageDescription")}</span>
                </div>
                <MacSelect
                  className="file-space-preferences-select"
                  value={languagePreference}
                  options={languageOptions}
                  ariaLabel={t("fileSpace.preferences.language.label")}
                  menuAlign="end"
                  menuMinWidth={222}
                  onChange={(preference) => {
                    setLanguageChoice(preference);
                    void setLanguagePreference(preference);
                  }}
                />
              </div>
              <div className="file-space-preferences-row">
                <div>
                  <strong>{t("fileSpace.preferences.center.showHelpButton")}</strong>
                  <span>{t("fileSpace.preferences.center.showHelpButtonDescription")}</span>
                </div>
                <label className="file-space-preferences-switch">
                  <input
                    type="checkbox"
                    aria-label={t("fileSpace.preferences.center.showHelpButton")}
                    checked={helpButtonVisible}
                    onChange={(event) => {
                      const visible = event.target.checked;
                      setHelpButtonVisibleState(visible);
                      setHelpButtonVisibility(visible);
                    }}
                  />
                  <span aria-hidden="true" />
                </label>
              </div>
            </div>
          </section>
        ) : activeSection === "appearance" ? (
          <section className="file-space-preferences-page" aria-labelledby="file-space-preferences-appearance-title">
            <header>
              <h3 id="file-space-preferences-appearance-title">{t("fileSpace.preferences.center.sections.appearance")}</h3>
              <p>{t("fileSpace.preferences.center.appearanceDescription")}</p>
            </header>

            <section className="file-space-preferences-section" aria-labelledby="file-space-preferences-theme-title">
              <div className="file-space-preferences-section-heading">
                <h4 id="file-space-preferences-theme-title">{t("fileSpace.preferences.center.theme")}</h4>
                <p>{t("fileSpace.preferences.center.themeDescription")}</p>
              </div>
              <div className="file-space-theme-options">
                {themes.map(({ key, icon: Icon }) => (
                  <button
                    className={`file-space-theme-option is-${key}${themePreference === key ? " is-selected" : ""}`}
                    type="button"
                    aria-pressed={themePreference === key}
                    key={key}
                    onClick={() => setTheme(key)}
                  >
                    <span className="file-space-theme-preview" aria-hidden="true">
                      <i />
                      <b />
                      <em><small /><small /><small /></em>
                    </span>
                    <span className="file-space-theme-option-label">
                      <Icon size={14} aria-hidden="true" />
                      {t(`fileSpace.preferences.appearance.${key}`)}
                    </span>
                    {themePreference === key ? <Check className="file-space-theme-option-check" size={14} aria-hidden="true" /> : null}
                  </button>
                ))}
              </div>
            </section>

            <section className="file-space-preferences-section" aria-labelledby="file-space-preferences-color-title">
              <div className="file-space-preferences-section-heading">
                <h4 id="file-space-preferences-color-title">{t("fileSpace.preferences.center.color")}</h4>
              </div>
              <div className="file-space-preferences-group">
                <div className="file-space-preferences-row">
                  <div>
                    <strong>{t("fileSpace.preferences.center.accentColor")}</strong>
                    <span>{t("fileSpace.preferences.center.accentDescription")}</span>
                  </div>
                  <label className="file-space-accent-color-control">
                    <input
                      type="color"
                      aria-label={t("fileSpace.preferences.center.accentColor")}
                      value={appearance.highlightColor}
                      onChange={(event) => setHighlightColor(event.target.value)}
                    />
                    <code>{appearance.highlightColor}</code>
                  </label>
                </div>
              </div>
            </section>

            <section className="file-space-preferences-section" aria-labelledby="file-space-preferences-type-title">
              <div className="file-space-preferences-section-heading">
                <h4 id="file-space-preferences-type-title">{t("fileSpace.preferences.center.typography")}</h4>
              </div>
              <div className="file-space-preferences-group">
                <div className="file-space-preferences-row">
                  <div>
                    <strong>{t("fileSpace.preferences.center.uiFont")}</strong>
                    <span>{t("fileSpace.preferences.center.uiFontDescription")}</span>
                  </div>
                  <MacSelect
                    className="file-space-preferences-select"
                    value={appearance.uiFont}
                    options={uiFontOptions}
                    ariaLabel={t("fileSpace.preferences.center.uiFont")}
                    menuAlign="end"
                    menuMinWidth={190}
                    onChange={setUiFont}
                  />
                </div>
                <div className="file-space-preferences-row">
                  <div>
                    <strong>{t("fileSpace.preferences.center.textSize")}</strong>
                  </div>
                  <MacSelect
                    className="file-space-preferences-size-select"
                    value={String(appearance.bodyFontSize)}
                    options={bodySizeOptions}
                    ariaLabel={t("fileSpace.preferences.center.textSize")}
                    menuAlign="end"
                    menuMinWidth={108}
                    onChange={(value) => setBodyFontSize(Number(value))}
                  />
                </div>
                <div className="file-space-preferences-row">
                  <div>
                    <strong>{t("fileSpace.preferences.center.codeSize")}</strong>
                  </div>
                  <MacSelect
                    className="file-space-preferences-size-select"
                    value={String(appearance.codeFontSize)}
                    options={codeSizeOptions}
                    ariaLabel={t("fileSpace.preferences.center.codeSize")}
                    menuAlign="end"
                    menuMinWidth={108}
                    onChange={(value) => setCodeFontSize(Number(value))}
                  />
                </div>
                <div className="file-space-preferences-row">
                  <div>
                    <strong>{t("fileSpace.preferences.center.fontSmoothing")}</strong>
                    <span>{t("fileSpace.preferences.center.fontSmoothingDescription")}</span>
                  </div>
                  <label className="file-space-preferences-switch">
                    <input
                      type="checkbox"
                      aria-label={t("fileSpace.preferences.center.fontSmoothing")}
                      checked={appearance.fontSmoothing}
                      onChange={(event) => setFontSmoothing(event.target.checked)}
                    />
                    <span aria-hidden="true" />
                  </label>
                </div>
              </div>
            </section>

            <div className="file-space-preferences-reset">
              <button
                type="button"
                onClick={() => {
                  setTheme("system");
                  resetAppearance();
                }}
              >
                <RotateCcw size={14} aria-hidden="true" />
                {t("fileSpace.preferences.center.resetAppearance")}
              </button>
            </div>
          </section>
        ) : null}

        {hasMountedAiService ? (
          <section
            className="file-space-preferences-page is-ai-service"
            aria-labelledby="file-space-preferences-ai-service-title"
            hidden={activeSection !== "aiService"}
          >
            <header>
              <h3 id="file-space-preferences-ai-service-title">{t("fileSpace.settings.aiServiceTitle")}</h3>
            </header>
            <AiServiceSettings
              embedded
              visible={activeSection === "aiService"}
              onCancel={onClose}
            />
          </section>
        ) : null}

        {activeSection === "background" ? (
          <section
            className="file-space-preferences-page is-background"
            aria-labelledby="file-space-preferences-background-title"
          >
            <header>
              <h3 id="file-space-preferences-background-title">{t("fileSpace.settings.backgroundTitle")}</h3>
            </header>
            <FileSpaceBackgroundTasks onOpenSemantic={onOpenSemantic} />
          </section>
        ) : null}
      </div>
    </div>
  );
}
