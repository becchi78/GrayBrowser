import { useState } from "react";
import { revealVideoInExplorer } from "./api";
import { errorText } from "./lib/errorText";

// 右クリックメニューの「パスをコピー」「エクスプローラーで開く」の実行。失敗の
// 理由は error に残し、次の操作で消す(表示のしかたは呼び出し側)。
export function usePathActions() {
  const [error, setError] = useState<string | null>(null);

  async function copyPath(path: string) {
    setError(null);
    try {
      await navigator.clipboard.writeText(path);
    } catch (e) {
      console.error(e);
      setError(`パスをコピーできませんでした: ${errorText(e)}`);
    }
  }

  async function reveal(videoId: string) {
    setError(null);
    try {
      await revealVideoInExplorer(videoId);
    } catch (e) {
      console.error(e);
      setError(`エクスプローラーで開けませんでした: ${errorText(e)}`);
    }
  }

  return { error, copyPath, reveal, clearError: () => setError(null) };
}
