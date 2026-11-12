import { useEffect, useState } from "react";
import { getThumbnails } from "./api";

// 重複パネルの行に出す1枚のサムネイル。videoId が変わるたびに取得し直し、
// 古い取得の結果は捨てる。取得できなければ null。
export function useFirstThumbnail(videoId: string): string | null {
  const [dataUrl, setDataUrl] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getThumbnails(videoId)
      .then((urls) => {
        if (cancelled) return;
        // 6枚配列の先頭1枚だけを表示する（重複パネルの行は1枚表示のみ）。
        setDataUrl(urls?.[0] ?? null);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [videoId]);

  return dataUrl;
}
