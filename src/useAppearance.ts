import { useCallback, useEffect, useRef, useState } from "react";
import { getAppearance, setAppearanceMode, setTheme, setWindowTheme } from "./api";
import {
  applyAppearance,
  browserStorage,
  parseMode,
  parseTheme,
  readStoredAppearance,
  writeStoredAppearance,
  type Appearance,
  type Mode,
  type Theme,
} from "./lib/appearance";

// テーマと外観の現在値を持ち、<html> の data-theme / data-mode、localStorage の
// 写し、ウィンドウ枠の明暗(setTheme)へ反映する。正は SQLite の設定で、
// 起動直後は index.html の同期スクリプトと同じ写しから始め、DB の値が
// 取れたら補正する。選択は即座に反映し、保存は後追いで行う。
export interface UseAppearance {
  appearance: Appearance;
  selectTheme: (theme: Theme) => void;
  selectMode: (mode: Mode) => void;
}

export function useAppearance(): UseAppearance {
  const [appearance, setAppearance] = useState<Appearance>(() =>
    readStoredAppearance(browserStorage()),
  );
  const current = useRef(appearance);
  // DB の値の取得が終わる前にユーザーが選んだ項目は、遅れて届く古い DB の値で
  // 上書きしない。
  const touched = useRef({ theme: false, mode: false });
  const nativeMode = useRef<Appearance["mode"] | null>(null);

  // 外部(DOM・localStorage・ウィンドウ枠)への反映。state は触らないので、
  // マウント時に同期的に呼んでも描画の連鎖を起こさない。
  const sync = useCallback((next: Appearance) => {
    current.current = next;
    applyAppearance(document.documentElement, next);
    writeStoredAppearance(browserStorage(), next);
    if (nativeMode.current !== next.mode) {
      nativeMode.current = next.mode;
      // 枠の色が揃わないだけで機能には影響しないので、失敗は記録に留める。
      setWindowTheme(next.mode).catch((e) => console.error(e));
    }
  }, []);

  const commit = useCallback(
    (next: Appearance) => {
      sync(next);
      setAppearance(next);
    },
    [sync],
  );

  useEffect(() => {
    sync(current.current);
    let cancelled = false;
    getAppearance()
      .then((stored) => {
        if (cancelled) return;
        // バックエンドが未知の値を返しても、属性には既知の値だけを入れる。
        const theme = parseTheme(stored.theme);
        const mode = parseMode(stored.mode);
        commit({
          theme: touched.current.theme || theme === null ? current.current.theme : theme,
          mode: touched.current.mode || mode === null ? current.current.mode : mode,
        });
      })
      .catch((e) => console.error(e));

    return () => {
      cancelled = true;
    };
  }, [sync, commit]);

  const selectTheme = useCallback(
    (theme: Theme) => {
      touched.current.theme = true;
      commit({ ...current.current, theme });
      setTheme(theme).catch((e) => console.error(e));
    },
    [commit],
  );

  const selectMode = useCallback(
    (mode: Mode) => {
      touched.current.mode = true;
      commit({ ...current.current, mode });
      setAppearanceMode(mode).catch((e) => console.error(e));
    },
    [commit],
  );

  return { appearance, selectTheme, selectMode };
}
