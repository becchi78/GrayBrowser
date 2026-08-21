import { RATING_FILTER_OPTIONS, parseRatingFilterValue } from "../lib/ratingBar";

interface Props {
  /** `null` = "すべて表示"(評価による絞り込みなし)。 */
  value: number | null;
  onChange: (v: number | null) => void;
}

// TagBarと並ぶ独立したフィルタ軸。タグ機構(TagBar/tags/video_tags)には一切
// 関与しない -- videos.ratingのみを対象とする単純な閾値フィルタ。
export function RatingBar({ value, onChange }: Props) {
  return (
    <div className="rating-bar" data-testid="rating-bar">
      <select
        value={value === null ? "" : String(value)}
        onChange={(e) => onChange(parseRatingFilterValue(e.target.value))}
        aria-label="評価で絞り込み"
        data-testid="rating-bar-select"
      >
        {RATING_FILTER_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </div>
  );
}
