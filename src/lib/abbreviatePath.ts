// 長いパスを、先頭（ドライブ・ルート）と末尾（フォルダ名・ファイル名）を残して
// 中間だけ省略する。パスは末尾ほど識別に効くため、末尾に多めの文字数を割り当てる。
// src/lib/boundedCache.ts と同じ型の純関数で、node --experimental-strip-types --test
// で単体テストする。
//
// 文字数はUTF-16コードユニットではなくコードポイントで数える（サロゲートペアを
// 途中で切って文字化けさせないため）。全角文字は表示幅が広いので、見た目の幅の
// 最終的な保険は呼び出し側のCSS（text-overflow: ellipsis）が担う。
export const PATH_ELLIPSIS = "…";

export function abbreviatePath(path: string, maxChars: number): string {
  const chars = Array.from(path);
  if (chars.length <= maxChars) {
    return path;
  }
  if (maxChars <= 1) {
    return PATH_ELLIPSIS;
  }
  const keep = maxChars - 1;
  const head = Math.floor(keep * 0.4);
  const tail = keep - head;
  return chars.slice(0, head).join("") + PATH_ELLIPSIS + chars.slice(chars.length - tail).join("");
}
