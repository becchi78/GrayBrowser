import { setRating } from "../api";

interface Props {
  videoId: string;
  value: number;
  onChange: (next: number) => void;
}

const STARS = [1, 2, 3, 4, 5];

export function StarRating({ videoId, value, onChange }: Props) {
  function handleSet(next: number) {
    setRating(videoId, next)
      .then(() => onChange(next))
      .catch(console.error);
  }

  return (
    <div
      className="star-rating"
      role="radiogroup"
      aria-label="評価"
      data-testid="video-row-rating"
    >
      {STARS.map((star) => (
        <button
          key={star}
          type="button"
          className={star <= value ? "star star--filled" : "star"}
          onClick={() => handleSet(star)}
          role="radio"
          aria-checked={star === value}
          aria-label={`星${star}`}
        >
          ★
        </button>
      ))}
      <button type="button" className="star-clear" onClick={() => handleSet(0)} disabled={value === 0}>
        評価をクリア
      </button>
    </div>
  );
}
