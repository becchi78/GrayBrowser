import { useEffect, useState } from "react";
import { listWatchFolders } from "./api";

// 登録フォルダが0件かどうか。空の一覧に、フォルダの登録先を案内するために使う。
// refreshKey が変わるたびに取り直す。
export function useHasNoWatchFolders(refreshKey: number): boolean {
  // Starts false so the "register a folder" hint never flashes before the
  // first fetch resolves; only a confirmed empty list shows it.
  const [noWatchFolders, setNoWatchFolders] = useState(false);

  useEffect(() => {
    let cancelled = false;
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) setNoWatchFolders(rows.length === 0);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  return noWatchFolders;
}
