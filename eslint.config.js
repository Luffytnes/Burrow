import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules"] },
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": "warn",
      // React Compiler optimization hints (react-hooks v7) — désactivés car l'app
      // n'utilise pas React Compiler. Ces règles ne détectent pas de bugs réels
      // sans le compilateur ; les activer uniquement si React Compiler est adopté.
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/refs": "off",
      "react-hooks/immutability": "off",
      "react-hooks/exhaustive-deps": "warn",
      "react-hooks/preserve-manual-memoization": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // Le code existant utilise volontiers les catch vides pour les invoke Tauri
      "no-empty": ["error", { allowEmptyCatch: true }],
    },
  }
);
