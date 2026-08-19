import { useEffect, useState } from "react";
import { listGenerationFailures, retryMetadataProbe, retryThumbnailGeneration } from "../api";
import { onCatalogChanged } from "../events";
import { splitDirAndFileName } from "../lib/paths";
import type { ExhaustedMetadataDto, ExhaustedThumbnailDto, GenerationFailuresDto } from "../types";

const EMPTY: GenerationFailuresDto = { thumbnail_failures: [], metadata_failures: [] };

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
