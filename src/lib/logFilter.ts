// ログ表示の絞り込み。解析(行 → エントリ)は Rust 側(gb_core::log_view)が担い、
// ここは受け取ったエントリの絞り込みと保持件数の管理だけを持つ。
// 絞り込みの挙動は gb_core::log_view::entry_matches と揃えること。

export const LOG_LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;

export interface LogEntryLike {
  level: string;
  target: string;
  message: string;
}

// レベル不明のエントリ(複数行メッセージの途中から読み始めた断片)は、
// レベルを判定できないので絞り込みで隠さない。隠すと文字が黙って消える。
export function entryMatches(
  entry: LogEntryLike,
  levels: ReadonlySet<string>,
  query: string,
): boolean {
  if (entry.level !== "UNKNOWN" && !levels.has(entry.level)) {
    return false;
  }
  const q = query.trim().toLowerCase();
  if (q === "") {
    return true;
  }
  return entry.message.toLowerCase().includes(q) || entry.target.toLowerCase().includes(q);
}

// reset のときは保持分を捨てて入れ替える。それ以外は末尾に足し、
// 画面に持つのは直近 max 件までにする(古い分は捨てる)。
export function mergeEntries<T>(
  prev: readonly T[],
  incoming: readonly T[],
  reset: boolean,
  max: number,
): T[] {
  const merged = reset ? [...incoming] : [...prev, ...incoming];
  return merged.length > max ? merged.slice(merged.length - max) : merged;
}

// 一覧は 1 エントリ 1 行で出すため、複数行メッセージの改行を記号に置き換える。
export function singleLine(message: string): string {
  return message.replace(/\r?\n/g, " ⏎ ");
}
