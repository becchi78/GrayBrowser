import { useEffect, useState } from "react";
import {
  countMissingMetadata,
  countMissingThumbnails,
  listGenerationFailures,
  regenerateMissingThumbnails,
  reprobeMissingMetadata,
  retryMetadataProbe,
  retryThumbnailGeneration,
} from "../api";
import { onCatalogChanged } from "../events";
import { ConfirmDialog } from "./ConfirmDialog";
import { splitDirAndFileName } from "../lib/paths";
import type { ExhaustedMetadataDto, ExhaustedThumbnailDto, GenerationFailuresDto } from "../types";

const EMPTY: GenerationFailuresDto = { thumbnail_failures: [], metadata_failures: [] };

type BulkKind = "thumbnail" | "metadata";

// Everything that differs between the two bulk actions; the click/confirm
// flow below is shared.
const BULK_ACTIONS: Record<
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

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge, not this component's own
  // toggle button. The component stays mounted regardless of `open` so
  // the fetch effect and the onCatalogChanged subscription below keep the
  // badge's count fresh in the background even while the panel is closed.
  open: boolean;
  // Reports the current total failure count up to StatusBar for the badge
  // label.
  onCountChange?: (count: number) => void;
}

export function GenerationFailuresPanel({ refreshKey, open, onCountChange }: Props) {
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

  function retryKey(kind: "thumbnail" | "metadata", id: string) {
    return `${kind}:${id}`;
  }

  async function handleRetryThumbnail(video: ExhaustedThumbnailDto) {
    const key = retryKey("thumbnail", video.id);
    setRetrying((prev) => new Set(prev).add(key));
    try {
      await retryThumbnailGeneration(video.id);
      // The success/failure of the retry itself surfaces via catalog:changed
      // (which triggers refresh() above), so this list is not updated
      // optimistically here.
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setRetrying((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  }

  async function handleRetryMetadata(video: ExhaustedMetadataDto) {
    const key = retryKey("metadata", video.id);
    setRetrying((prev) => new Set(prev).add(key));
    try {
      await retryMetadataProbe(video.id);
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setRetrying((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  }

  async function handleBulkClick(kind: BulkKind) {
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

  async function handleBulkConfirm() {
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

  const totalFailures = failures.thumbnail_failures.length + failures.metadata_failures.length;

  useEffect(() => {
    onCountChange?.(totalFailures);
    // Fires whenever `failures` (and therefore totalFailures) changes.
    // `onCountChange` is always a stable useState setter passed by
    // StatusBar.
  }, [totalFailures, onCountChange]);

  if (!open) {
    return null;
  }

  return (
    <section className="generation-failures-panel status-panel" data-testid="status-panel">
      <div className="generation-failures-bulk">
        <button
          type="button"
          data-testid="regenerate-missing-thumbnails-btn"
          onClick={() => handleBulkClick("thumbnail")}
          disabled={counting || running}
        >
          未作成のサムネイルをすべて再作成
        </button>
        <button
          type="button"
          data-testid="reprobe-missing-metadata-btn"
          onClick={() => handleBulkClick("metadata")}
          disabled={counting || running}
        >
          メタデータをすべて再取得
        </button>
        {notice && (
          <p className="generation-failures-notice" data-testid="regenerate-missing-notice">
            {notice}
          </p>
        )}
      </div>
      <ConfirmDialog
        open={pending !== null}
        title={pending ? BULK_ACTIONS[pending.kind].title : ""}
        confirmLabel={pending ? BULK_ACTIONS[pending.kind].confirmLabel : ""}
        busy={running}
        onConfirm={handleBulkConfirm}
        onCancel={() => setPending(null)}
      >
        {pending && <p>{BULK_ACTIONS[pending.kind].body(pending.count)}</p>}
      </ConfirmDialog>
      {error && <p className="generation-failures-error">{error}</p>}
      {totalFailures === 0 ? (
        <p>生成に失敗したファイルはありません。</p>
      ) : (
        <ul className="generation-failures-list">
          {failures.thumbnail_failures.map((video) => (
            <li key={`thumbnail-${video.id}`} className="generation-failure-item">
              <span className="generation-failure-badge">サムネイル生成失敗</span>
              <div className="generation-failure-info">
                <span className="generation-failure-name">{video.file_name}</span>
                <FilePathRow filePath={video.file_path} />
                <span className="generation-failure-attempts">
                  失敗回数: {video.thumbnail_attempts}
                </span>
              </div>
              <button
                type="button"
                onClick={() => handleRetryThumbnail(video)}
                disabled={retrying.has(retryKey("thumbnail", video.id))}
              >
                再試行
              </button>
            </li>
          ))}
          {failures.metadata_failures.map((video) => (
            <li key={`metadata-${video.id}`} className="generation-failure-item">
              <span className="generation-failure-badge">メタデータ取得失敗</span>
              <div className="generation-failure-info">
                <span className="generation-failure-name">{video.file_name}</span>
                <FilePathRow filePath={video.file_path} />
                <span className="generation-failure-attempts">
                  失敗回数: {video.metadata_attempts}
                </span>
              </div>
              <button
                type="button"
                onClick={() => handleRetryMetadata(video)}
                disabled={retrying.has(retryKey("metadata", video.id))}
              >
                再試行
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

// Shared by both the thumbnail-failure and metadata-failure `<li>`
// blocks above (identical markup, just a different source array), so the
// splitDirAndFileName call site exists once rather than twice. See
// DuplicateGroupsPanel's own (single-use, so left inline as an IIFE) call
// site for the same pattern and App.css's `.file-path-row` comment for why
// the split happens at all.
function FilePathRow({ filePath }: { filePath: string }) {
  const { dir, name } = splitDirAndFileName(filePath);
  return (
    <span className="file-path-row" title={filePath}>
      <span className="file-path-dir">{dir}</span>
      <span className="file-path-file">{name}</span>
    </span>
  );
}
