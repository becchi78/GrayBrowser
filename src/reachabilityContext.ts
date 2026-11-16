import { createContext, useContext } from "react";
import { isFolderUnreachable, isPathUnreachable } from "./lib/reachability";

// 今到達できない監視フォルダの一覧。ReachabilityProvider が供給する。
// Provider の外（テストなど）では「到達不能なし」として扱う。
export const UnreachableFoldersContext = createContext<readonly string[]>([]);

/** 今到達できない監視フォルダの一覧(メニューなど、行の外で判定したいとき用)。 */
export function useUnreachableFolders(): readonly string[] {
  return useContext(UnreachableFoldersContext);
}

/** `path` が今到達できないフォルダの配下にあるか。 */
export function useIsPathUnreachable(path: string): boolean {
  const folders = useContext(UnreachableFoldersContext);
  return isPathUnreachable(path, folders);
}

/** `folder`(監視フォルダ)自身が今到達できないか。 */
export function useIsFolderUnreachable(folder: string): boolean {
  const folders = useContext(UnreachableFoldersContext);
  return isFolderUnreachable(folder, folders);
}
