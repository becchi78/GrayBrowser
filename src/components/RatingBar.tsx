import { RATING_FILTER_OPTIONS, toggleRating } from "../lib/ratingBar";

interface Props {
  /** 有効な評価。空 = "ALL"(評価による絞り込みなし)。 */
  value: number[];
  onChange: (v: number[]) => void;
}

// タグ部品と並ぶ独立したフィルタ軸。タグ機構(TagBar/tags/video_tags)には一切
// 関与しない -- videos.ratingのみを対象に、有効な評価のいずれかと一致する動画だけを表示する単純なフィルタ。
export function RatingBar({ value, onChange }: Props) {
  const allActive = value.length === 0;
  return (
    <div className="rating-bar" data-testid="rating-bar" role="group" aria-label="評価で絞り込み">
      <button
        type="button"
        className={allActive ? "rating-bar-item rating-bar-item--active" : "rating-bar-item"}
        data-testid="rating-bar-item"
        data-rating-value=""
        aria-pressed={allActive}
        onClick={() => onChange([])}
      >
        ALL
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
          </button>
        );
      })}
    </div>
  );
}
