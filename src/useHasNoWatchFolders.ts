import { useWatchFolderList } from "./useWatchFolderList";

// 登録フォルダが0件かどうか。空の一覧に、フォルダの登録先を案内するために使う。
// refreshKey が変わるたびに取り直す。
export function useHasNoWatchFolders(refreshKey: number): boolean {
  // The "register a folder" hint never flashes before the first fetch
  // resolves; only a confirmed empty list (not null) shows it.
  const folders = useWatchFolderList(refreshKey);
  return folders !== null && folders.length === 0;
}
