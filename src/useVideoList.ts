import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listVideos } from "./api";
import { onCatalogChanged } from "./events";
import { createLatestOnlyGuard } from "./lib/latestRequest";
import type { SortDirection, SortField, VideoDto } from "./types";

// Max re-fetch frequency while catalog:changed events are arriving.
const NOTIFY_THROTTLE_MS = 300;

/** 一覧に何をどの順で出すか。どれかが変わると取り直す。 */
export interface VideoListQuery {
  search: string;
  sortField: SortField;
  sortDirection: SortDirection;
  tagIds: number[];
  folderPath: string | null;
  ratings: number[];
}

// 一覧に出す動画(取得と catalog:changed での取り直し)。取り直しが反映される
// たびに onLoaded(rows) を、一覧を差し替えたのと同じ続きの中で呼ぶ。React が
// 一覧の差し替えと onLoaded の中の state 更新を1回の描画にまとめるので、
// 呼び出し側はそこで一覧に合わせた後始末(選択の整理など)を、余分な描画や
// 通知を挟まずに済ませられる。
export function useVideoList(
  { search, sortField, sortDirection, tagIds, folderPath, ratings }: VideoListQuery,
  refreshKey: number,
  onLoaded: (rows: VideoDto[]) => void,
) {
  const [videos, setVideos] = useState<VideoDto[]>([]);
  // 一覧を再取得するたびに増える。VideoDtoには詳細情報の取得状況が含まれず、
  // 行は自分で状況を知る手段がないため、「一覧が更新された」ことだけを行に
  // 伝えて、取得が終わっていない行に取り直させる。
  const [catalogVersion, setCatalogVersion] = useState(0);

  // onLoaded は取得の effect の依存に入れない。入れると、呼び出し側が描画の
  // たびに作り直す関数を渡しただけで、購読のし直しと取り直しが毎回起きる。
  // 応答が届いた時点の最新を ref から読む。
  const onLoadedRef = useRef(onLoaded);
  useLayoutEffect(() => {
    onLoadedRef.current = onLoaded;
  });

  // Re-fetches once on mount/refreshKey change, then on every catalog:changed
  // event from the backend (no polling).
  //
  // A burst of events is throttled to at most one re-fetch per
  // NOTIFY_THROTTLE_MS, as a *throttle* rather than a trailing-edge debounce:
  // the first event fires immediately and re-fetches keep happening during
  // the burst, so the grid fills in progressively during a large batch
  // instead of staying frozen until everything finishes.
  useEffect(() => {
    let cancelled = false;
    // 一覧の再取得は応答の順序が入れ替わりうる。古い要求の応答（まだ生成中の
    // 状態）が新しい一覧を上書きしないよう、最新の要求の応答だけを反映する。
    // effect の再実行時の古い応答は cancelled で破棄されるので、ガードは
    // effect ごとに作れば足りる。
    const guard = createLatestOnlyGuard();

    const tick = async () => {
      const requestId = guard.begin();
      try {
        const rows = await listVideos({
          search,
          sortField,
          sortDirection,
          tagIds,
          folderPath,
          ratings,
        });
        if (cancelled || !guard.isLatest(requestId)) return;
        setVideos(rows);
        setCatalogVersion((v) => v + 1);
        onLoadedRef.current(rows);
      } catch (e) {
        console.error(e);
      }
    };
    tick();

    let lastRun = 0;
    let trailingTimer: ReturnType<typeof setTimeout> | null = null;
    let trailingPending = false;

    const scheduleTick = () => {
      const elapsed = Date.now() - lastRun;
      if (elapsed >= NOTIFY_THROTTLE_MS) {
        lastRun = Date.now();
        trailingPending = false;
        tick();
        return;
      }
      trailingPending = true;
      if (trailingTimer === null) {
        trailingTimer = setTimeout(() => {
          trailingTimer = null;
          if (trailingPending) {
            trailingPending = false;
            lastRun = Date.now();
            tick();
          }
        }, NOTIFY_THROTTLE_MS - elapsed);
      }
    };

    const unlisten = onCatalogChanged(scheduleTick);

    return () => {
      cancelled = true;
      unlisten();
      if (trailingTimer !== null) clearTimeout(trailingTimer);
    };
    // catalog:changed re-fetches always use the latest search/sort/tagIds/
    // folderPath/ratings via closure, so those need to be dependencies too
    // (not just refreshKey).
  }, [refreshKey, search, sortField, sortDirection, tagIds, folderPath, ratings]);

  return { videos, catalogVersion };
}
