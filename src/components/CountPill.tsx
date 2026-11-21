import { formatCount } from "../lib/filterChips";

// 左ペインの項目の右端に出す件数。全テーマで表示する。件数は補助の情報で
// 項目名は変わらないので、支援技術には読ませない。
export function CountPill({ count }: { count: number | undefined }) {
  if (count === undefined) return null;
  return (
    <span className="count-pill" data-testid="count-pill" aria-hidden="true">
      {formatCount(count)}
    </span>
  );
}
