import { useEffect, useState } from "react";
import { getLibraryCounts } from "./api";
import { onCatalogChanged } from "./events";
import type { LibraryCounts } from "./types";

// カタログの変化(スキャン・サムネイル生成の進捗・評価の変更)は短時間に
// 何度も届くので、取り直しは最後の通知からこの時間だけ待つ(絞り込み中バーの
// タグ名の取り直しも同じ値を使う)。
export const REFETCH_DELAY_MS = 500;

// フィルタに関わらない全体件数・フォルダ別件数・レーティング別件数。
// 最初の取得が終わるまで(失敗時は取得できないまま)は null を返す。
// 「0件」と「未取得」を区別し、未取得の間は件数の表示を出さない。
export function useLibraryCounts(refreshKey: number): LibraryCounts | null {
  const [counts, setCounts] = useState<LibraryCounts | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () => {
      getLibraryCounts()
        .then((c) => {
          if (!cancelled) setCounts(c);
        })
        .catch(console.error);
    };
    load();
    const unlisten = onCatalogChanged(() => {
      clearTimeout(timer);
      timer = setTimeout(load, REFETCH_DELAY_MS);
    });
    return () => {
      cancelled = true;
      clearTimeout(timer);
      unlisten();
    };
  }, [refreshKey]);

  return counts;
}
