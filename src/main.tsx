import React, { useState, useEffect } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { LangContext } from "./i18n/useT";
import { locales, LangKey } from "./i18n/locales";
import { ThemeContext, ThemeMode } from "./ThemeContext";

function initialThemeMode(): ThemeMode {
  const saved = localStorage.getItem("theme");
  if (saved === "light" || saved === "dark" || saved === "auto") return saved;
  // Migration depuis l'ancien réglage booléen "dark"
  const legacy = localStorage.getItem("dark");
  if (legacy !== null) {
    const mode: ThemeMode = legacy === "true" ? "dark" : "light";
    localStorage.setItem("theme", mode);
    localStorage.removeItem("dark");
    return mode;
  }
  return "auto";
}

export function Root() {
  const savedLang = (localStorage.getItem("lang") ?? "fr") as LangKey;
  const [lang, setLangState] = useState<LangKey>(savedLang in locales ? savedLang : "fr");

  const [mode, setModeState] = useState<ThemeMode>(initialThemeMode);
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches
  );

  const setLang = (l: LangKey) => {
    localStorage.setItem("lang", l);
    setLangState(l);
  };
  const setMode = (m: ThemeMode) => {
    localStorage.setItem("theme", m);
    setModeState(m);
  };

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const dark = mode === "auto" ? systemDark : mode === "dark";

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);

  return (
    <ThemeContext.Provider value={{ mode, setMode, dark }}>
      <LangContext.Provider value={{ lang, setLang, t: locales[lang].t }}>
        <ErrorBoundary>
          <App />
        </ErrorBoundary>
      </LangContext.Provider>
    </ThemeContext.Provider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
