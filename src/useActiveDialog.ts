import { useCallback, useEffect, useState } from "react";
import { hasUndismissibleModal } from "./undismissibleModals";

export type DialogKind = "folder" | "wbImport" | "tagBar" | "about" | "log" | "settings" | "update";

// メニューから開くダイアログは同時に1つだけ。別のダイアログを開くと、開いていたものは
// 閉じる。ただし閉じられないダイアログ(処理中など)が開いている間は、差し替えると
// 状態が壊れるので、開く要求を無視する。
export function useActiveDialog() {
  const [active, setActive] = useState<DialogKind | null>(null);

  const open = useCallback((kind: DialogKind) => {
    if (hasUndismissibleModal()) return;
    setActive(kind);
  }, []);

  // 自分が開いているときだけ閉じる。すでに別のダイアログに切り替わった後で遅れて
  // 呼ばれた onClose が、新しいダイアログまで閉じないようにする。
  const close = useCallback((kind: DialogKind) => {
    setActive((current) => (current === kind ? null : current));
  }, []);

  return { active, open, close };
}

// ネイティブメニューの項目に応じてダイアログを開く購読。購読ごとに別の effect に
// するため、呼び出し側でダイアログの数だけ呼ぶ。
export function useMenuOpen(
  subscribe: (callback: () => void) => () => void,
  kind: DialogKind,
  open: (kind: DialogKind) => void,
) {
  useEffect(() => subscribe(() => open(kind)), [subscribe, kind, open]);
}
