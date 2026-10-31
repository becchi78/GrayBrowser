// ファイルパスから、そのファイルのあるフォルダのパスを取り出す。
// src/lib/abbreviatePath.ts と同じ型の純関数で、node --experimental-strip-types --test
// で単体テストする。
//
// splitDirAndFileName（paths.ts）は末尾の区切りを含めて返すため、"C:\" と "C:" を
// 区別する必要がある表示用途には合わない。ここではフォルダ名として自然な形
// （末尾の区切りなし。ただしドライブ直下は "C:\" のまま）で返す。
// UNC（\\server\share\dir\a.mp4）は先頭の \\ を壊さないよう、最後の区切りで
// 切るだけにしている。区切りが無い/空のときは空文字を返す。
export function parentFolder(filePath: string): string {
  const trimmed = filePath.replace(/[\\/]+$/, "");
  const lastSep = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  if (lastSep === -1) {
    return "";
  }
  const dir = trimmed.slice(0, lastSep);
  if (/^[A-Za-z]:$/.test(dir)) {
    return `${dir}${trimmed[lastSep]}`;
  }
  return dir;
}
