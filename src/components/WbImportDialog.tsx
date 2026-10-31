import { useEffect, useState } from "react";
import { openWbImportLog, pickWbFile, pickWbThumbnailFolder, startWbImport } from "../api";
import {
  onWbImportComplete,
  onWbImportFailed,
  onWbImportProgress,
  onWbImportStage,
} from "../events";
import { abbreviatePath } from "../lib/abbreviatePath";
import { buildWbImportResultDisplay } from "../lib/wbImportResultDisplay";
import {
  computeWbImportSteps,
  formatStepCount,
  isWbImportStageId,
  progressBarValue,
} from "../lib/wbImportSteps";
import type { WbImportStageCounts } from "../lib/wbImportSteps";
import type { WbImportStageId, WbImportSummary } from "../types";

interface Props {
  onClose: () => void;
  onImportComplete: () => void;
}

// 表示用に省略するパスの最大文字数。全体は title で見られる。
const PATH_DISPLAY_MAX_CHARS = 60;

function PathLabel({ path }: { path: string }) {
  return (
    <span className="wb-import-path" title={path}>
      {abbreviatePath(path, PATH_DISPLAY_MAX_CHARS)}
    </span>
  );
}

// 完了時・失敗時の両方に置く。ログには実際のファイル名・パスが入るので、
// 不具合報告に添付する場合の注意を添える。
function ImportLogLink() {
  const [openError, setOpenError] = useState<string | null>(null);

  async function handleOpenLog() {
    setOpenError(null);
    try {
      await openWbImportLog();
    } catch (e) {
      console.error(e);
      setOpenError(String(e));
    }
  }

  return (
    <div>
      <button type="button" data-testid="import-open-log-btn" onClick={handleOpenLog}>
        ログを確認する
      </button>
      <p className="wb-import-result-detail">
        ログには実際のファイル名・パスが含まれます。不具合報告に添付するときはご注意ください。
      </p>
      {openError && (
        <p className="wb-import-error" data-testid="import-open-log-error">
          {openError}
        </p>
      )}
    </div>
  );
}

// The parent mounts this only while the dialog is open (see App.tsx), so
// every open starts from the file-selection step: all state below is
// discarded on close.
export function WbImportDialog({ onClose, onImportComplete }: Props) {
  const [wbPath, setWbPath] = useState<string | null>(null);
  const [thumbnailFolderPath, setThumbnailFolderPath] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [counts, setCounts] = useState<WbImportStageCounts>({});
  const [stage, setStage] = useState<WbImportStageId | null>(null);
  const [summary, setSummary] = useState<WbImportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showUnmatchedDetail, setShowUnmatchedDetail] = useState(false);

  useEffect(() => {
    const unlistenProgress = onWbImportProgress((p) => {
      // 未知の段階名は段階イベントと同様に無視する。
      if (isWbImportStageId(p.stage)) {
        setCounts((prev) => ({ ...prev, [p.stage]: { processed: p.processed, total: p.total } }));
      }
    });
    const unlistenStage = onWbImportStage((st) => {
      // 未知の段階名(将来の追加など)は無視し、表示を壊さない。
      if (isWbImportStageId(st)) {
        setStage(st);
      }
    });
    const unlistenComplete = onWbImportComplete((s) => {
      setSummary(s);
      setImporting(false);
      onImportComplete();
    });
    const unlistenFailed = onWbImportFailed((reason) => {
      setError(reason);
      setImporting(false);
    });

    return () => {
      unlistenProgress();
      unlistenStage();
      unlistenComplete();
      unlistenFailed();
    };
    // onImportComplete is expected to be referentially stable across
    // App.tsx's lifetime (a setState updater), so it's intentionally
    // omitted to avoid re-subscribing on every render (same reasoning the
    // old WbImportPanel this dialog replaces used).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function requestClose() {
    // Closing mid-import would unmount this component (and with it, its
    // wb_import:* event subscriptions above) while the backend import
    // thread keeps running unattended -- there would be no way to learn the
    // outcome short of reopening and hoping the events already fired.
    // Simplest guard: importing must finish (or fail) before the dialog can
    // be dismissed, mirroring the close button's own `disabled={importing}`.
    if (!importing) {
      onClose();
    }
  }

  async function handlePickWbFile() {
    try {
      const path = await pickWbFile();
      if (path !== null) {
        setWbPath(path);
      }
    } catch (e) {
      console.error(e);
    }
  }

  async function handlePickThumbnailFolder() {
    try {
      const path = await pickWbThumbnailFolder();
      if (path !== null) {
        setThumbnailFolderPath(path);
      }
    } catch (e) {
      console.error(e);
    }
  }

  async function handleStartImport() {
    if (wbPath === null || thumbnailFolderPath === null) {
      return;
    }
    setImporting(true);
    setCounts({});
    setStage(null);
    setSummary(null);
    setError(null);
    setShowUnmatchedDetail(false);
    try {
      // Always register the imported videos' folders: without them in the
      // sidebar the migration is left half-done.
      await startWbImport(wbPath, thumbnailFolderPath, true);
    } catch (e) {
      console.error(e);
      setError(String(e));
      setImporting(false);
    }
  }

  const bar = progressBarValue(stage, counts);
  const resultDisplay = summary ? buildWbImportResultDisplay(summary) : null;

  return (
    // No overlay click-to-close: a stray click outside would throw away the
    // result or a half-made selection; only the close button closes.
    <div className="dialog-overlay">
      <div className="dialog wb-import-dialog" data-testid="import-dialog">
        <h2>.wbインポート</h2>

        {!summary && !error && (
          <>
            <div className="wb-import-dialog-row">
              <button type="button" onClick={handlePickWbFile} disabled={importing}>
                .wbファイルを選択
              </button>
              {wbPath && <PathLabel path={wbPath} />}
            </div>
            <div className="wb-import-dialog-row">
              <button type="button" onClick={handlePickThumbnailFolder} disabled={importing}>
                旧サムネイルフォルダを選択
              </button>
              {thumbnailFolderPath && <PathLabel path={thumbnailFolderPath} />}
            </div>
            <div className="wb-import-dialog-row">
              <button
                type="button"
                onClick={handleStartImport}
                disabled={importing || wbPath === null || thumbnailFolderPath === null}
              >
                {importing ? "インポート中..." : "インポート"}
              </button>
            </div>
          </>
        )}

        {importing && (
          <div className="wb-import-steps">
            <ol className="wb-import-steps-list">
              {computeWbImportSteps(stage, null).map((s) => {
                const countText = formatStepCount(s.status, counts[s.key]);
                return (
                  <li key={s.key} className={`wb-import-step wb-import-step--${s.status}`}>
                    {s.label}
                    {countText && <span className="wb-import-step-count">{countText}</span>}
                  </li>
                );
              })}
            </ol>
            {bar && (
              <progress
                data-testid="import-progress-bar"
                className="wb-import-progress-bar"
                value={bar.value}
                max={bar.max}
              />
            )}
          </div>
        )}

        {summary && resultDisplay && (
          <div className="wb-import-result">
            <p className="wb-import-result-line" data-testid="import-result-total">
              {resultDisplay.totalText}
            </p>
            <p
              className="wb-import-result-line wb-import-result-success"
              data-testid="import-result-success"
            >
              {resultDisplay.registeredText}
            </p>
            {resultDisplay.offlineThumbnailsText && (
              <p className="wb-import-result-line" data-testid="import-result-offline-thumbnails">
                {resultDisplay.offlineThumbnailsText}
              </p>
            )}
            {resultDisplay.watchFoldersAddedText && (
              <p className="wb-import-result-line">{resultDisplay.watchFoldersAddedText}</p>
            )}
            {resultDisplay.skippedText && (
              <p className="wb-import-result-line" data-testid="import-result-skipped">
                {resultDisplay.skippedText}
              </p>
            )}

            {resultDisplay.attention.map((item) => {
              const testId =
                item.key === "unmatched"
                  ? "import-result-needs-review"
                  : item.key === "invalid-name"
                    ? "import-result-skipped-invalid-name"
                    : item.key === "failed-rows"
                      ? "import-result-failed-rows"
                      : undefined;
              return (
                <div
                  key={item.key}
                  className={`wb-import-result-attention wb-import-result-attention--${item.tone}`}
                  data-testid={testId}
                >
                  <p className="wb-import-result-line">{item.text}</p>
                  {item.expandable && (
                    <>
                      <button type="button" onClick={() => setShowUnmatchedDetail((v) => !v)}>
                        確認する
                      </button>
                      {showUnmatchedDetail && (
                        // A real per-file list/manual-relink UI is out of scope
                        // here. This only surfaces where the unmatched files
                        // still are.
                        <p className="wb-import-result-detail">
                          対象の旧サムネイルファイルは、選択した旧サムネイルフォルダ内に残っています。個別の一覧表示・手動での紐付け操作は今後のアップデートで対応予定です。
                        </p>
                      )}
                    </>
                  )}
                </div>
              );
            })}

            {(resultDisplay.details.length > 0 || resultDisplay.watchFolders.length > 0) && (
              <details className="wb-import-result-details">
                <summary>詳細</summary>
                <ul className="wb-import-result-details-list">
                  {resultDisplay.details.map((item) => (
                    <li key={item.key}>{item.text}</li>
                  ))}
                  {resultDisplay.watchFolders.length > 0 && (
                    <li data-testid="import-result-watch-folders">
                      監視フォルダに追加:
                      <ul className="wb-import-result-watch-folder-list">
                        {resultDisplay.watchFolders.map((folder) => (
                          <li key={folder}>{folder}</li>
                        ))}
                      </ul>
                    </li>
                  )}
                </ul>
              </details>
            )}

            <ImportLogLink />
          </div>
        )}

        {error && (
          <>
            <p className="wb-import-error">移行に失敗しました: {error}</p>
            <ImportLogLink />
          </>
        )}

        <div className="dialog-footer">
          <button
            type="button"
            data-testid="import-dialog-close-btn"
            onClick={requestClose}
            disabled={importing}
          >
            閉じる
          </button>
        </div>
      </div>
    </div>
  );
}
