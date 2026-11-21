import { useEffect, useState } from "react";
import { getDeveloperMode } from "../api";
import { DEVELOPER_MODE_CHANGED_EVENT } from "../lib/developerMode";
import "./DevModeBadge.css";

// 開発者モードがオンの間、常に表示する。読み込み時に設定を取得し、設定の保存後に更新する。
// 取得に失敗したときは出さない(誤って「オン」と示すより、出さないほうが安全)。
export function DevModeBadge() {
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

  return enabled ? (
    <span className="dev-mode-badge" data-testid="dev-mode-badge">
      開発者モード
    </span>
  ) : null;
}
