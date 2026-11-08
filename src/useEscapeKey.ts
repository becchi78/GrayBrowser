import { useEffect } from "react";

// enabled が false の間はリスナーを登録しない(閉じてはいけない状態で
// Escape を握りつぶさないため)。preventDefault するのは、フォーカス中の
// 入力欄などが Escape を別の用途に使うのを防ぐため。
export function useEscapeKey(onEscape: () => void, enabled: boolean = true) {
  useEffect(() => {
    if (!enabled) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onEscape();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onEscape, enabled]);
}
