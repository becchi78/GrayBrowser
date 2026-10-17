import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

const DIALOG_MESSAGE =
  "tauri-plugin-dialog が window.confirm を非同期関数に差し替え、Promise を返すため、確認が機能しない。自前のモーダル(ConfirmDialog)を使うこと";

export default tseslint.config(
  // "target" (not "src-tauri/target"): PR1's Cargo workspace split moved the
  // build output to the repo root (ci.yml's build job has the same note).
  { ignores: ["dist", "target"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      // tauri-plugin-dialog が window.confirm/alert/prompt を非同期関数に
      // 差し替え、Promise を返す。Promise は常に真なので `if (!confirmed)` が
      // 効かず、確認なしで処理が進んでしまう（型検査でも検出できない）。
      "no-restricted-properties": [
        "error",
        ...["confirm", "alert", "prompt"].map((property) => ({
          object: "window",
          property,
          message: DIALOG_MESSAGE,
        })),
      ],
      "no-restricted-globals": [
        "error",
        ...["confirm", "alert", "prompt"].map((name) => ({ name, message: DIALOG_MESSAGE })),
      ],
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
);
