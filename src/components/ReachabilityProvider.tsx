import { useEffect, useState, type ReactNode } from "react";
import { getUnreachableFolders } from "../api";
import { onReachabilityChanged } from "../events";
import { UnreachableFoldersContext } from "../reachabilityContext";

/**
 * バックエンドが把握している「今到達できない監視フォルダ」を購読して、
 * 配下の VideoRow / DuplicateGroupsPanel に配る。
 *
 * 購読を先に張ってから初期値を取得する。取得中に届いたイベントの方が新しい
 * ので、その場合は取得結果で上書きしない。
 */
export function ReachabilityProvider({ children }: { children: ReactNode }) {
  const [folders, setFolders] = useState<readonly string[]>([]);

  useEffect(() => {
    let gotEvent = false;
    let cancelled = false;
    const unlisten = onReachabilityChanged((next) => {
      gotEvent = true;
      setFolders(next);
    });
    getUnreachableFolders()
      .then((initial) => {
        if (!cancelled && !gotEvent) setFolders(initial);
      })
      .catch((e) => console.error("failed to load unreachable folders:", e));
    return () => {
      cancelled = true;
      unlisten();
    };
  }, []);

  return (
    <UnreachableFoldersContext.Provider value={folders}>
      {children}
    </UnreachableFoldersContext.Provider>
  );
}
