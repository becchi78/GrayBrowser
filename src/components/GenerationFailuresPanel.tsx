import { ConfirmDialog } from "./ConfirmDialog";
import { GenerationFailureItem } from "./GenerationFailureItem";
import { BULK_ACTIONS, useGenerationFailures } from "../useGenerationFailures";

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge, not this component's own
  // toggle button. The component stays mounted regardless of `open` so
  // the fetch effect and the onCatalogChanged subscription in
  // useGenerationFailures keep the badge's count fresh in the background
  // even while the panel is closed.
  open: boolean;
  // Reports the current total failure count up to StatusBar for the badge
  // label.
  onCountChange?: (count: number) => void;
}

export function GenerationFailuresPanel({ refreshKey, open, onCountChange }: Props) {
  const {
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
  } = useGenerationFailures(refreshKey, onCountChange);

  if (!open) {
    return null;
  }

  return (
    <section className="generation-failures-panel status-panel" data-testid="status-panel">
      <div className="generation-failures-bulk">
        <button
          type="button"
          data-testid="regenerate-missing-thumbnails-btn"
          onClick={() => requestBulk("thumbnail")}
          disabled={counting || running}
        >
          未作成のサムネイルをすべて再作成
        </button>
        <button
          type="button"
          data-testid="reprobe-missing-metadata-btn"
          onClick={() => requestBulk("metadata")}
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
        onConfirm={confirmBulk}
        onCancel={cancelBulk}
      >
        {pending && <p>{BULK_ACTIONS[pending.kind].body(pending.count)}</p>}
      </ConfirmDialog>
      {error && <p className="generation-failures-error">{error}</p>}
      {totalFailures === 0 ? (
        <p>生成に失敗したファイルはありません。</p>
      ) : (
        <ul className="generation-failures-list">
          {failures.thumbnail_failures.map((video) => (
            <GenerationFailureItem
              key={`thumbnail-${video.id}`}
              badge="サムネイル生成失敗"
              fileName={video.file_name}
              filePath={video.file_path}
              attempts={video.thumbnail_attempts}
              onRetry={() => retryThumbnail(video)}
              disabled={isRetrying("thumbnail", video.id)}
            />
          ))}
          {failures.metadata_failures.map((video) => (
            <GenerationFailureItem
              key={`metadata-${video.id}`}
              badge="メタデータ取得失敗"
              fileName={video.file_name}
              filePath={video.file_path}
              attempts={video.metadata_attempts}
              onRetry={() => retryMetadata(video)}
              disabled={isRetrying("metadata", video.id)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
