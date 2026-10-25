// 到達不能フォルダ配下のパスかどうかの判定。バックエンドの
// `gb_core::paths::is_under_folder` と同じ規則に揃えてある（大文字小文字を
// 区別せず、`/` と `\` を同じ区切りとして、フォルダ境界で一致させる）。
// 規則を変えるときは両方を同時に直すこと。

function comparable(path: string): string {
  return path.toLowerCase().split("/").join("\\");
}

/**
 * `path` が `folder` の内側にあるか。`W:\video2\a.mp4` は `W:\video` の配下では
 * ない（文字列の前方一致だけでは誤判定になるため、区切り文字の境界を要求する）。
 * `folder` の末尾の区切りの有無は問わない。空のフォルダは何も含まない。
 */
export function isUnderFolder(path: string, folder: string): boolean {
  const base = comparable(folder).replace(/\\+$/, "");
  if (base.trim() === "") return false;
  return comparable(path).startsWith(base + "\\");
}

/** `path` が `unreachableFolders` のどれかの配下か。 */
export function isPathUnreachable(path: string, unreachableFolders: readonly string[]): boolean {
  return unreachableFolders.some((folder) => isUnderFolder(path, folder));
}
