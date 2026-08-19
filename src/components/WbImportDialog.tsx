import { useEffect, useState } from "react";
import { pickWbFile, pickWbThumbnailFolder, startWbImport } from "../api";
import { onWbImportComplete, onWbImportFailed, onWbImportProgress } from "../events";
import { classifyTagsAssigned, type TagsAssignedTone } from "../lib/wbImportTagsDisplay";
import type { WbImportProgress, WbImportSummary } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
  onImportComplete: () => void;
}

type StepKey = "parse" | "link" | "rebuild";

const STEPS: { key: StepKey; label: string }[] = [
  { key: "parse", label: "解析" },
  { key: "link", label: "紐付け" },
  { key: "rebuild", label: "UUID再構築" },
];

export function WbImportDialog({ open, onClose, onImportComplete }: Props) {
  const [wbPath, setWbPath] = useState<string | null>(null);
  const [thumbnailFolderPath, setThumbnailFolderPath] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [progress, setProgress] = useState<WbImportProgress | null>(null);
  const [summary, setSummary] = useState<WbImportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showUnmatchedDetail, setShowUnmatchedDetail] = useState(false);

  useEffect(() => {
    const unlistenProgress = onWbImportProgress((p) => setProgress(p));
    const unlistenComplete = onWbImportComplete((s) => {
      setSummary(s);
      setImporting(false);
      setProgress(null);
      onImportComplete();
    });
    const unlistenFailed = onWbImportFailed((reason) => {
      setError(reason);
      setImporting(false);
      setProgress(null);
    });

    return () => {
      unlistenProgress();
      unlistenComplete();
      unlistenFailed();
    };
    // onImportComplete is expected to be referentially stable across
    // App.tsx's lifetime (a setState updater), so it's intentionally
    // omitted to avoid re-subscribing on every render (same reasoning the
    // old WbImportPanel this dialog replaces used).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!open) {
    return null;
  }

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
    setProgress(null);
    setSummary(null);
    setError(null);
    setShowUnmatchedDetail(false);
    try {
      await startWbImport(wbPath, thumbnailFolderPath);
    } catch (e) {
      console.error(e);
      setError(String(e));
      setImporting(false);
    }
  }

  // The 3-step checklist (解析→紐付け→UUID再構築) does not map 1:1 onto the
  // actual backend pipeline (src-tauri/src/wb_import/pipeline.rs's
  // `import_all`): UUID assignment happens inline, per row, during the very
  // same loop that reports wb_import:progress -- there is no distinct
  // "UUID再構築" phase at all. The only other real phase (`link_thumbnails`,
  // the legacy-JPG linking pass) runs silently after that loop finishes,
  // with no progress event of its own. Given only `progress.processed`/
  // `total` and whether `summary`/`error` has arrived to work with, this is
  // a simplified, approximate visual only:
  //  - "解析": active while processed < total.
  //  - "紐付け"/"UUID再構築" (bundled, since "UUID再構築" has no independent
  //    signal -- splitting it further would just be inventing data this
  //    dialog doesn't have): active once processed === total but the final
  //    summary/error hasn't arrived yet (this is really `link_thumbnails`'s
  //    window).
  function stepStatus(step: StepKey): "pending" | "active" | "done" {
    if (summary || error) {
      return "done";
    }
    if (!importing) {
      return "pending";
    }
    const rowLoopFinished =
      progress !== null && progress.total > 0 && progress.processed >= progress.total;
    if (step === "parse") {
      return rowLoopFinished ? "done" : "active";
    }
    return rowLoopFinished ? "active" : "pending";
  }

  // tags_assigned===0 になる原因(元データにタグが無かった/全行
  // 既登録スキップ/推定失敗/真の異常)を区別する表示ロジックは
  // src/lib/wbImportTagsDisplay.ts の classifyTagsAssigned に切り出し済み。
  // ここではトーンをCSSクラスへマッピングするだけ。"success"は既存の見た目
  // (元々クラス無指定だった)を変えないため意図的にクラスを付けない。
  function tagsAssignedClassName(tone: TagsAssignedTone): string {
    if (tone === "success") {
      return "wb-import-result-line";
    }
    return `wb-import-result-line wb-import-result-${tone}`;
  }

  function renderTagsAssignedLine(s: WbImportSummary) {
    const { tone, text } = classifyTagsAssigned(s);
    return <p className={tagsAssignedClassName(tone)}>{text}</p>;
  }

  return (
    <div className="dialog-overlay" onClick={requestClose}>
      <div
        className="dialog wb-import-dialog"
        data-testid="import-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>.wbインポート</h2>

        {!summary && !error && (
          <>
            <div className="wb-import-dialog-row">
              <button type="button" onClick={handlePickWbFile} disabled={importing}>
                .wbファイルを選択
              </button>
              {wbPath && <span className="wb-import-path">{wbPath}</span>}
            </div>
            <div className="wb-import-dialog-row">
              <button type="button" onClick={handlePickThumbnailFolder} disabled={importing}>
                旧サムネイルフォルダを選択
              </button>
              {thumbnailFolderPath && <span className="wb-import-path">{thumbnailFolderPath}</span>}
            </div>
            <div className="wb-import-dialog-row">
              <button
                type="button"
                onClick={handleStartImport}
                disabled={importing || wbPath === null || thumbnailFolderPath === null}
              >
                {importing ? "インポート中..." : "移行開始"}
              </button>
            </div>
          </>
        )}

        {importing && (
          <div className="wb-import-steps">
            <ol className="wb-import-steps-list">
              {STEPS.map((s) => (
                <li key={s.key} className={`wb-import-step wb-import-step--${stepStatus(s.key)}`}>
                  {s.label}
                </li>
              ))}
            </ol>
            <progress
              data-testid="import-progress-bar"
              className="wb-import-progress-bar"
              value={progress?.processed ?? 0}
              max={progress && progress.total > 0 ? progress.total : 1}
            />
            {progress && (
              <span className="wb-import-progress">
                {progress.processed} / {progress.total}
              </span>
            )}
          </div>
        )}

        {summary && (
          <div className="wb-import-result">
            <p
              className="wb-import-result-line wb-import-result-success"
              data-testid="import-result-success"
            >
              正常インポート: {summary.registered}件
            </p>

            {summary.thumbnails_unmatched > 0 ? (
              <div
                className="wb-import-result-needs-review"
                data-testid="import-result-needs-review"
              >
                <p className="wb-import-result-line">
                  自動対応できなかった旧サムネイル: {summary.thumbnails_unmatched}件 —
                  エラーではありません。手動で紐付けてください。
                </p>
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
              </div>
            ) : (
              <p className="wb-import-result-line wb-import-result-muted">
                自動対応できなかった旧サムネイル: 0件
              </p>
            )}

            {summary.thumbnails_failed > 0 ? (
              <p className="wb-import-result-line wb-import-result-failed">
                旧サムネイル変換に失敗: {summary.thumbnails_failed}件
              </p>
            ) : (
              <p className="wb-import-result-line wb-import-result-muted">
                旧サムネイル変換の失敗: 0件
              </p>
            )}

            {renderTagsAssignedLine(summary)}

            <p className="wb-import-result-line wb-import-result-muted">
              スキップ (登録済み): {summary.skipped}件 / score丸め: {summary.clamped_scores}件 /
              旧サムネイル紐付け成功: {summary.thumbnails_linked}件
            </p>
            <p className="wb-import-note">
              オンライン動画のサムネイルは動画ファイルから別途バックグラウンドで自動生成されます。
            </p>
          </div>
        )}

        {error && <p className="wb-import-error">移行に失敗しました: {error}</p>}

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
