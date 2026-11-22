import { useDeveloperMode } from "../useDeveloperMode";
import { DataDirIndicator } from "./DataDirIndicator";
import "./DataDirIndicator.css";
import "./DevModeBadge.css";

// 開発者モードがオンの間、常に表示する。取得に失敗したときは出さない
// (誤って「オン」と示すより、出さないほうが安全)。使用中のデータフォルダも、
// 開発者モードがオンのときだけ、バッジの左に並べる。
export function DevModeBadge() {
  const enabled = useDeveloperMode();

  return enabled ? (
    <>
      <DataDirIndicator />
      <span className="dev-mode-badge" data-testid="dev-mode-badge">
        開発者モード
      </span>
    </>
  ) : null;
}
