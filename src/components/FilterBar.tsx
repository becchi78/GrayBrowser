import { useEffect, useState } from "react";
import { listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import { REFETCH_DELAY_MS } from "../useLibraryCounts";
import { buildFilterChips, formatCount, hasActiveFilter, type FilterChip } from "../lib/filterChips";

interface Props {
  folderPath: string | null;
  onFolderPathChange: (folderPath: string | null) => void;
  tagIds: number[];
  onTagIdsChange: (tagIds: number[]) => void;
  ratings: number[];
  onRatingsChange: (ratings: number[]) => void;
  /** 絞り込み後の件数(一覧の行数)。 */
  filteredCount: number;
  /** 絞り込みなしの総数。未取得の間は null で、件数の表示を出さない。 */
  totalCount: number | null;
  /** タグの編集で名前が変わりうるので、親が編集の保存のたびに増やす。 */
  tagBarRefreshKey: number;
}

const KIND_LABELS: Record<FilterChip["kind"], string> = {
  folder: "フォルダ",
  tag: "タグ",
  rating: "レーティング",
};

// 検索ボックスの下に置く「絞り込み中」バー。選択中のフォルダ・タグ・★を
// チップで並べ、×で1つずつ、「すべて解除」でまとめて外す。何も絞り込んで
// いなければ描画しない(その分だけ一覧が広がる)。左ペインの選択 UI の代わりではなく、
// 同じ状態(親の state)を別の入口から外せるようにするもの。
export function FilterBar({
  folderPath,
  onFolderPathChange,
  tagIds,
  onTagIdsChange,
  ratings,
  onRatingsChange,
  filteredCount,
  totalCount,
  tagBarRefreshKey,
}: Props) {
  const [tagNames, setTagNames] = useState<ReadonlyMap<number, string>>(new Map());

  // タグ名が要るのはタグで絞り込んでいる間だけ。そうでなければ取得も購読もしない。
  const needsTagNames = tagIds.length > 0;

  useEffect(() => {
    if (!needsTagNames) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reload = () => {
      listAllTags()
        .then((tags) => {
          if (!cancelled) setTagNames(new Map(tags.map((t) => [t.id, t.name])));
        })
        .catch(console.error);
    };
    reload();
    // スキャンやサムネイル生成の進捗で通知が連続するので、最後の通知から
    // REFETCH_DELAY_MS だけ待って1回だけ取り直す。
    const unlisten = onCatalogChanged(() => {
      clearTimeout(timer);
      timer = setTimeout(reload, REFETCH_DELAY_MS);
    });
    return () => {
      cancelled = true;
      clearTimeout(timer);
      unlisten();
    };
  }, [tagBarRefreshKey, needsTagNames]);

  const selection = { folderPath, tagIds, ratings };
  if (!hasActiveFilter(selection)) return null;
  const chips = buildFilterChips(selection, tagNames);

  function remove(chip: FilterChip) {
    if (chip.kind === "folder") onFolderPathChange(null);
    else if (chip.kind === "tag") onTagIdsChange(tagIds.filter((id) => id !== chip.id));
    else onRatingsChange(ratings.filter((r) => r !== chip.value));
  }

  function clearAll() {
    onFolderPathChange(null);
    onTagIdsChange([]);
    onRatingsChange([]);
  }

  return (
    <div className="filter-bar" data-testid="filter-bar" role="group" aria-label="絞り込み中">
      <span className="filter-bar-title">絞り込み中</span>
      <ul className="filter-bar-chips" aria-live="polite" aria-relevant="additions removals">
        {chips.map((chip) => {
          const key = chip.kind === "folder" ? "folder" : `${chip.kind}-${chipId(chip)}`;
          return (
            <li key={key} className="filter-bar-chip" data-testid="filter-bar-chip" data-kind={chip.kind}>
              <span className="filter-bar-chip-kind">{KIND_LABELS[chip.kind]}</span>
              <span className="filter-bar-chip-label" title={chip.label}>
                {chip.label}
              </span>
              <button
                type="button"
                className="filter-bar-chip-remove"
                data-testid="filter-bar-chip-remove"
                aria-label={`${KIND_LABELS[chip.kind]} ${chip.label} の絞り込みを解除`}
                onClick={() => remove(chip)}
              >
                ×
              </button>
            </li>
          );
        })}
      </ul>
      <button
        type="button"
        className="link-button filter-bar-clear-all"
        data-testid="filter-bar-clear-all"
        onClick={clearAll}
      >
        すべて解除
      </button>
      {totalCount !== null && (
        <span
          className="filter-bar-count"
          data-testid="filter-bar-count"
          role="status"
          aria-live="polite"
        >
          <strong>{formatCount(filteredCount)}</strong> 件 / 全 {formatCount(totalCount)} 件
        </span>
      )}
    </div>
  );
}

function chipId(chip: Exclude<FilterChip, { kind: "folder" }>): number {
  return chip.kind === "tag" ? chip.id : chip.value;
}
