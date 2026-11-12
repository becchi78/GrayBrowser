import { useEffect, useState } from "react";
import {
  countMissingMetadata,
  countMissingThumbnails,
  listGenerationFailures,
  regenerateMissingThumbnails,
  reprobeMissingMetadata,
  retryMetadataProbe,
  retryThumbnailGeneration,
} from "./api";
import { onCatalogChanged } from "./events";
import { withoutItem } from "./lib/withoutKey";
import type { ExhaustedMetadataDto, ExhaustedThumbnailDto, GenerationFailuresDto } from "./types";

const EMPTY: GenerationFailuresDto = { thumbnail_failures: [], metadata_failures: [] };

type BulkKind = "thumbnail" | "metadata";

// Everything that differs between the two bulk actions; the click/confirm
// flow in useGenerationFailures is shared.
export const BULK_ACTIONS: Record<
  BulkKind,
  {
    count: () => Promise<number>;
    run: () => Promise<number>;
    emptyNotice: string;
    title: string;
    confirmLabel: string;
    body: (count: number) => string;
  }
> = {
  thumbnail: {
    count: countMissingThumbnails,
    run: regenerateMissingThumbnails,
    emptyNotice: "再作成が必要なサムネイルはありません。",
    title: "サムネイルの再作成",
    confirmLabel: "再作成する",
    body: (count) => `${count} 件の動画のサムネイルを再作成します。時間がかかることがあります。`,
  },
  metadata: {
    count: countMissingMetadata,
    run: reprobeMissingMetadata,
    emptyNotice: "再取得が必要なメタデータはありません。",
    title: "メタデータの再取得",
    confirmLabel: "再取得する",
    body: (count) => `${count} 件の動画のメタデータ(解像度など)を再取得します。`,
  },
};

function retryKey(kind: BulkKind, id: string) {
  return `${kind}:${id}`;
}

// 生成失敗パネルの状態と操作(一覧の取得と catalog:changed の購読、1件ずつの
// 再試行、一括の数える→確認→実行、件数の通知)。パネルを閉じていても呼び出し側は
// マウントを保つので、取得・購読・件数の通知は閉じていても動く。閉じても確認待ちの
// 一括操作は捨てず、開き直すと確認がそのまま出る。
export function useGenerationFailures(
  refreshKey: number,
  onCountChange?: (count: number) => void,
) {
  const [failures, setFailures] = useState<GenerationFailuresDto>(EMPTY);
  const [error, setError] = useState<string | null>(null);
  const [retrying, setRetrying] = useState<Set<string>>(new Set());

  // null = dialog closed; otherwise which bulk action awaits confirmation and
  // how many videos it covers.
  const [pending, setPending] = useState<{ kind: BulkKind; count: number } | null>(null);
  const [counting, setCounting] = useState(false);
  const [running, setRunning] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  function refresh() {
    listGenerationFailures()
      .then(setFailures)
      .catch((e) => {
        console.error(e);
        setError(String(e));
      });
  }

  useEffect(() => {
    refresh();
  }, [refreshKey]);

  useEffect(() => {
    const unlisten = onCatalogChanged(() => refresh());
    return () => {
      unlisten();
    };
  }, []);

  async function runRetry(key: string, retry: () => Promise<unknown>) {
    setRetrying((prev) => new Set(prev).add(key));
    try {
      await retry();
      // The success/failure of the retry itself surfaces via catalog:changed
      // (which triggers refresh() above), so this list is not updated
      // optimistically here.
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setRetrying((prev) => withoutItem(prev, key));
    }
  }

  function isRetrying(kind: BulkKind, id: string) {
    return retrying.has(retryKey(kind, id));
  }

  function retryThumbnail(video: ExhaustedThumbnailDto) {
    return runRetry(retryKey("thumbnail", video.id), () => retryThumbnailGeneration(video.id));
  }

  function retryMetadata(video: ExhaustedMetadataDto) {
    return runRetry(retryKey("metadata", video.id), () => retryMetadataProbe(video.id));
  }

  async function requestBulk(kind: BulkKind) {
    setNotice(null);
    setCounting(true);
    try {
      const count = await BULK_ACTIONS[kind].count();
      if (count === 0) {
        setNotice(BULK_ACTIONS[kind].emptyNotice);
      } else {
        setPending({ kind, count });
      }
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setCounting(false);
    }
  }

  async function confirmBulk() {
    if (!pending) return;
    setRunning(true);
    try {
      // Returns right away; progress shows in the status bar and the list
      // refreshes through catalog:changed.
      await BULK_ACTIONS[pending.kind].run();
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setRunning(false);
      setPending(null);
    }
  }

  function cancelBulk() {
    setPending(null);
  }

  const totalFailures = failures.thumbnail_failures.length + failures.metadata_failures.length;

  useEffect(() => {
    onCountChange?.(totalFailures);
    // Fires whenever `failures` (and therefore totalFailures) changes.
    // `onCountChange` is always a stable useState setter passed by
    // StatusBar.
  }, [totalFailures, onCountChange]);

  return {
    failures,
    totalFailures,
    error,
    notice,
    pending,
    counting,
    running,
    isRetrying,
    retryThumbnail,
    retryMetadata,
    requestBulk,
    confirmBulk,
    cancelBulk,
  };
}
