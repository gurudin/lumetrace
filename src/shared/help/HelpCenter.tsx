import { BookOpen, CircleHelp, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { helpArticles, searchHelp } from "./helpContent";
import {
  getHelpButtonVisibility,
  helpButtonVisibilityChangeEvent,
} from "./helpButtonVisibility";
import "./help-center.css";

/** A non-modal, offline guide shared by both application entries. */
export function HelpCenter() {
  const { t, i18n } = useTranslation();
  const language = i18n.resolvedLanguage ?? "en-US";
  const [visible, setVisible] = useState(getHelpButtonVisibility);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState("start");
  const trigger = useRef<HTMLButtonElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const reader = useRef<HTMLElement>(null);
  const articles = useMemo(() => helpArticles(language), [language]);
  const matches = useMemo(() => searchHelp(articles, query), [articles, query]);
  const article = matches.find(item => item.id === selected) ?? matches[0];
  const close = () => {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  };
  useEffect(() => {
    if (open) search.current?.focus({ preventScroll: true });
  }, [open]);
  useEffect(() => {
    const updateVisibility = () => setVisible(getHelpButtonVisibility());
    window.addEventListener(helpButtonVisibilityChangeEvent, updateVisibility);
    window.addEventListener("storage", updateVisibility);
    return () => {
      window.removeEventListener(helpButtonVisibilityChangeEvent, updateVisibility);
      window.removeEventListener("storage", updateVisibility);
    };
  }, []);
  useEffect(() => {
    if (!visible && open) setOpen(false);
  }, [open, visible]);
  useEffect(() => { reader.current?.scrollTo(0, 0); }, [article?.id, language]);
  if (!visible) return null;
  return <div className="help-center" data-help-center onKeyDown={event => {
    // Editing and navigation here must not act on selected workspace files.
    event.stopPropagation();
    if (event.key === "Escape" && open && !event.nativeEvent.isComposing) {
      event.preventDefault();
      close();
    }
  }} onDrop={event => { event.preventDefault(); event.stopPropagation(); }}>
    <div className="help-center-launcher">
      <button ref={trigger} type="button" className="help-center-trigger" aria-label={t("help.title")}
        aria-expanded={open} aria-controls="lumetrace-help" aria-haspopup="dialog"
        onClick={() => open ? close() : setOpen(true)}>
        <CircleHelp size={21} strokeWidth={1.7} aria-hidden="true" />
      </button>
      {!open && <span className="help-center-tooltip" role="tooltip">{t("help.title")}</span>}
    </div>
    <section id="lumetrace-help" className="help-center-panel" role="dialog" aria-modal="false"
      aria-labelledby="help-center-title" hidden={!open}>
      <header className="help-center-header">
        <BookOpen size={20} aria-hidden="true" />
        <div><h1 id="help-center-title">{t("help.title")}</h1><p>{t("help.subtitle")}</p></div>
        <button type="button" aria-label={t("help.close")} title={t("help.close")} onClick={close}><X size={18} aria-hidden="true" /></button>
      </header>
      <div className="help-center-search">
        <Search size={16} aria-hidden="true" />
        <input ref={search} type="search" value={query} maxLength={200} aria-label={t("help.search")}
          placeholder={t("help.placeholder")} onChange={event => { setQuery(event.target.value); setSelected(""); }} />
        {query && <button type="button" aria-label={t("help.clear")} onClick={() => { setQuery(""); search.current?.focus(); }}><X size={15} aria-hidden="true" /></button>}
      </div>
      <div className="help-center-body">
        <nav className="help-center-topics" aria-label={t("help.topics")}>
          <p role="status">{t("help.count", { count: matches.length })}</p>
          {matches.map(item => <button type="button" key={item.id} aria-current={article?.id === item.id ? "page" : undefined}
            onClick={() => { setSelected(item.id); reader.current?.focus({ preventScroll: true }); }}>
            {item.title}
          </button>)}
        </nav>
        <article ref={reader} className="help-center-reader" tabIndex={0} aria-label={article?.title ?? t("help.empty")}
          lang={language === "zh-CN" ? "zh-CN" : "en"}>
          {article ? <>
            {language !== "zh-CN" && language !== "en-US" && <p className="help-center-language" lang={language}>{t("help.languageNote")}</p>}
            <h2>{article.title}</h2><p className="help-center-summary">{article.summary}</p>
            {article.sections.map(section => <section key={section.title}>
              <h3>{section.title}</h3>
              <ol>{section.steps.map(step => <li key={step}>{step}</li>)}</ol>
            </section>)}
          </> : <div className="help-center-empty" lang={language}><Search size={26} aria-hidden="true" /><h2>{t("help.empty")}</h2><p>{t("help.emptyHint")}</p>
            <button type="button" onClick={() => { setQuery(""); search.current?.focus(); }}>{t("help.clear")}</button>
          </div>}
        </article>
      </div>
      <footer className="help-center-footer"><span>{t("help.offline")}</span><span>LumeTrace</span></footer>
    </section>
  </div>;
}
