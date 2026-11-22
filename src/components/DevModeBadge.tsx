import { useDeveloperMode } from "../useDeveloperMode";
import "./DevModeBadge.css";

// 開発者モードがオンの間、常に表示する。取得に失敗したときは出さない
// (誤って「オン」と示すより、出さないほうが安全)。
export function DevModeBadge() {
  const enabled = useDeveloperMode();

  return enabled ? (
    <span className="dev-mode-badge" data-testid="dev-mode-badge">
      開発者モード
    </span>
  ) : null;
}
