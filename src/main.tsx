import React, { useState, useEffect, Component, ErrorInfo, ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
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

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { error: null };
  }
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("App crash:", error, info);
  }
  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            height: "100vh",
            padding: "2rem",
            fontFamily: "monospace",
            background: "#131110",
            color: "#f2ede8",
          }}
        >
          <div style={{ fontSize: 14, color: "#f88779", marginBottom: 12, fontWeight: 700 }}>
            Erreur de rendu
          </div>
          <pre
            style={{
              fontSize: 11,
              color: "#aaa",
              maxWidth: 600,
              whiteSpace: "pre-wrap",
              textAlign: "left",
            }}
          >
            {this.state.error.message}
          </pre>
          <button
            onClick={() => this.setState({ error: null })}
            style={{
              marginTop: 20,
              padding: "6px 16px",
              background: "#e54857",
              color: "#ffffff",
              border: "none",
              borderRadius: 6,
              cursor: "pointer",
            }}
          >
            Réessayer
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

function Root() {
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
