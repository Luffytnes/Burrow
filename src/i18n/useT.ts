import { createContext, useContext } from "react";
import { locales, LangKey, Translations } from "./locales";

export const LangContext = createContext<{
  lang: LangKey;
  setLang: (l: LangKey) => void;
  t: Translations;
}>({
  lang: "en",
  setLang: () => {},
  t: locales["en"].t,
});

export const useT = () => useContext(LangContext);
