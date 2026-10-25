import { createContext, useContext } from "react";
import { isPathUnreachable } from "./lib/reachability";

// 今到達できない監視フォルダの一覧。ReachabilityProvider が供給する。
// Provider の外（テストなど）では「到達不能なし」として扱う。
export const UnreachableFoldersContext = createContext<readonly string[]>([]);

/** `path` が今到達できないフォルダの配下にあるか。 */
export function useIsPathUnreachable(path: string): boolean {
  const folders = useContext(UnreachableFoldersContext);
  return isPathUnreachable(path, folders);
}
