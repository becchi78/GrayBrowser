import { useEffect, type Dispatch, type SetStateAction } from "react";

// ネイティブメニューの項目に応じてダイアログを開く購読。購読ごとに別の
// effect にするため、呼び出し側でダイアログの数だけ呼ぶ。
export function useMenuOpen(
  subscribe: (callback: () => void) => () => void,
  setOpen: Dispatch<SetStateAction<boolean>>,
) {
  useEffect(() => {
    const unlisten = subscribe(() => setOpen(true));
    return () => unlisten();
  }, [subscribe, setOpen]);
}
