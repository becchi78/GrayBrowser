import { RATING_FILTER_OPTIONS, toggleRating } from "../lib/ratingBar";
import type { LibraryCounts } from "../types";
import { CountPill } from "./CountPill";

interface Props {
  /** 有効な評価。空 = "ALL"(評価による絞り込みなし)。 */
  value: number[];
  onChange: (v: number[]) => void;
  /** 件数のピル。省略・未取得の間は出さない。 */
  counts?: LibraryCounts | null;
}

// タグ部品と並ぶ独立したフィルタ軸。タグ機構(TagBar/tags/video_tags)には一切
// 関与しない -- videos.ratingのみを対象に、有効な評価のいずれかと一致する動画だけを表示する単純なフィルタ。
export function RatingBar({ value, onChange, counts }: Props) {
  const allActive = value.length === 0;
  const ratingCounts = new Map(counts?.ratings.map((r) => [r.rating, r.count]));
  return (
    <div className="rating-bar" data-testid="rating-bar" role="group" aria-label="レーティングで絞り込み">
      <button
        type="button"
        className={allActive ? "rating-bar-item rating-bar-item--active" : "rating-bar-item"}
        data-testid="rating-bar-item"
        data-rating-value=""
        aria-pressed={allActive}
        onClick={() => onChange([])}
      >
        ALL
        <CountPill count={counts?.total} />
      </button>
      {RATING_FILTER_OPTIONS.map((opt) => {
        const active = value.includes(opt.value);
        return (
          <button
            key={opt.value}
            type="button"
            className={active ? "rating-bar-item rating-bar-item--active" : "rating-bar-item"}
            data-testid="rating-bar-item"
            data-rating-value={String(opt.value)}
            aria-pressed={active}
            onClick={() => onChange(toggleRating(value, opt.value))}
          >
            {opt.label}
            {counts && <CountPill count={ratingCounts.get(opt.value) ?? 0} />}
          </button>
        );
      })}
    </div>
  );
}
