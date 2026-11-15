import { useEffect, useState } from "react";
import { listWatchFolders } from "./api";

// 登録フォルダの一覧。refreshKey が変わるたびに取り直す。最初の取得が終わる
// まで(失敗時は取得できないまま)は null を返す。「0件」と「未取得」を区別
// できるようにして、呼び出し側が未取得の間に空の表示を出さないようにする。
export function useWatchFolderList(refreshKey: number | undefined): string[] | null {
  const [folders, setFolders] = useState<string[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) setFolders(rows);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  return folders;
}
