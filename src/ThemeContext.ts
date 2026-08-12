import { createContext, useContext } from "react";

export type ThemeMode = "light" | "dark" | "auto";

export interface ThemeCtxType {
  /** Préférence choisie par l'utilisateur */
  mode: ThemeMode;
  setMode: (m: ThemeMode) => void;
  /** Thème effectivement appliqué (résout "auto" via le système) */
  dark: boolean;
}

export const ThemeContext = createContext<ThemeCtxType>({
  mode: "auto",
  setMode: () => {},
  dark: false,
});

export const useTheme = () => useContext(ThemeContext);
