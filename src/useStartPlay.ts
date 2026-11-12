import { useState } from "react";
import { playVideo } from "./api";

// 重複パネルの行からの再生。blocked のあいだと開始中は起動せず、開始中の状態と
// 失敗理由を返す。
export function useStartPlay(filePath: string, blocked: boolean) {
  const [playError, setPlayError] = useState<string | null>(null);
  const [isStartingPlay, setIsStartingPlay] = useState(false);

  // 再生の開始は外部プレーヤーの起動待ち（ネットワーク先では数十秒）になりうる。
  // 再生ボタンの disabled で待機を示せないため、開始中の二重ダブルクリックは
  // 無視し、結果が出るまでの状態と失敗理由を行内に表示する。
  async function handlePlay() {
    if (blocked || isStartingPlay) return;
    setPlayError(null);
    setIsStartingPlay(true);
    try {
      await playVideo(filePath);
    } catch (e) {
      console.error(e);
      setPlayError(`再生に失敗しました: ${String(e)}`);
    } finally {
      setIsStartingPlay(false);
    }
  }

  return { playError, isStartingPlay, handlePlay };
}
