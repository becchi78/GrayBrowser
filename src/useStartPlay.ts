import { useState } from "react";
import { playVideo } from "./api";

// 一覧の行と重複パネルの行に共通の、ダブルクリックからの再生。blocked のあいだと
// 開始中は起動せず、開始中の状態と失敗理由を返す（どう表示するかは呼び出し側）。
export function useStartPlay(filePath: string, blocked: boolean) {
  const [playError, setPlayError] = useState<string | null>(null);
  const [isStartingPlay, setIsStartingPlay] = useState(false);

  // 再生の開始は外部プレーヤーの起動待ち（ネットワーク先では数十秒）になりうる。
  // どちらの行も再生ボタンを持たず disabled で待機を示せないため、開始中の
  // 二重ダブルクリックはここで無視し、バックグラウンドの起動処理を積み上げない。
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
