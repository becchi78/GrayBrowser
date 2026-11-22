import { useEffect, useState } from "react";
import { getDeveloperMode } from "./api";
import { DEVELOPER_MODE_CHANGED_EVENT } from "./lib/developerMode";

// 開発者モードの現在値。読み込み時に取得し、設定の保存を知らせるイベントで取得し直す。
// 取得に失敗したときは false(誤って「オン」とみなして開発者向けの項目を出すより安全)。
export function useDeveloperMode(): boolean {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      getDeveloperMode()
        .then((value) => {
          if (!cancelled) {
            setEnabled(value);
          }
        })
        .catch((e) => {
          console.error(e);
          if (!cancelled) {
            setEnabled(false);
          }
        });
    };
    load();
    window.addEventListener(DEVELOPER_MODE_CHANGED_EVENT, load);
    return () => {
      cancelled = true;
      window.removeEventListener(DEVELOPER_MODE_CHANGED_EVENT, load);
    };
  }, []);

  return enabled;
}
