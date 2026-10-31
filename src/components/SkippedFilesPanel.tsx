import { useCallback, useEffect, useState } from "react";
import { listSkippedFiles, renameSkippedFile } from "../api";
import { splitDirAndFileName } from "../lib/paths";
import type { SkippedFileDto } from "../types";

// The only reason code emitted today (crates/gb-core/src/scan_pipeline.rs),
// but kept as a lookup with a fallback so a future reason doesn't render
// blank.
const REASON_LABELS: Record<string, string> = {
  machine_dependent_char: "機種依存文字を含むファイル名",
};

// 同名ファイルをフォルダで見分けられるよう、パス全体を1つの連続したテキストとして出す。
function SkippedFilePath({ filePath }: { filePath: string }) {
  const { dir, name } = splitDirAndFileName(filePath);
  return (
    <>
      <span className="skipped-file-dir">{dir}</span>
      <span className="skipped-file-name">{name}</span>
    </>
  );
}

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge (a single "which panel is
  // open" selection), not this component's own toggle button. The
  // component stays mounted regardless of `open` so its fetch effect below
  // keeps the badge's count fresh in the background even while the panel
  // is closed.
  open: boolean;
  // Reports the current count up to StatusBar for the badge label.
  onCountChange?: (count: number) => void;
}

export function SkippedFilesPanel({ refreshKey, open, onCountChange }: Props) {
  const [files, setFiles] = useState<SkippedFileDto[]>([]);
  // Only one row is edited at a time; opening another row's editor replaces
  // this, so a stale draft/error never lingers on a different row.
  const [editingId, setEditingId] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(
    () =>
      listSkippedFiles()
        .then((f) => {
          setFiles(f);
          onCountChange?.(f.length);
        })
        .catch(console.error),
    // `onCountChange` is always a stable useState setter passed by
    // StatusBar, so including it here does not cause extra refetches.
    [onCountChange],
  );

  useEffect(() => {
    void reload();
  }, [refreshKey, reload]);

  const startEdit = (f: SkippedFileDto) => {
    if (busy) return;
    setEditingId(f.id);
    setDraft(f.file_name);
    setError(null);
    setNotice(null);
  };

  const cancelEdit = () => {
    if (busy) return;
    setEditingId(null);
    setError(null);
  };

  const submit = async (f: SkippedFileDto) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await renameSkippedFile(f.id, draft);
      setEditingId(null);
      setNotice(
        result.registered
          ? null
          : "名前は変更しました。登録は次回のスキャンで行われます。",
      );
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return null;
  }

  return (
    <section className="skipped-files-panel status-panel" data-testid="status-panel">
      {notice && (
        <p className="skipped-files-notice" role="status">
          {notice}
        </p>
      )}
      {files.length === 0 ? (
        <p>未登録ファイルはありません。</p>
      ) : (
        <>
          <p className="skipped-files-guidance">
            「名前を変更」で機種依存文字を取り除いた名前に変えると、そのまま通常の動画として登録されます。
            エクスプローラーで直した場合は、次回のスキャンで登録されます。
          </p>
          <ul className="skipped-files-list">
            {files.map((f) => (
              <li key={f.id}>
                {editingId === f.id ? (
                  <div className="skipped-file-edit">
                    <input
                      className="skipped-file-edit-input"
                      type="text"
                      value={draft}
                      disabled={busy}
                      autoFocus
                      aria-label="新しいファイル名"
                      onChange={(e) => setDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.nativeEvent.isComposing) return;
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void submit(f);
                        } else if (e.key === "Escape") {
                          e.preventDefault();
                          cancelEdit();
                        }
                      }}
                    />
                    <button type="button" disabled={busy} onClick={() => void submit(f)}>
                      OK
                    </button>
                    <button type="button" disabled={busy} onClick={cancelEdit}>
                      キャンセル
                    </button>
                    {error && (
                      <p className="skipped-file-edit-error" role="alert">
                        {error}
                      </p>
                    )}
                  </div>
                ) : (
                  <>
                    <SkippedFilePath filePath={f.file_path} />
                    <span className="skipped-file-reason">
                      {REASON_LABELS[f.reason] ?? f.reason}
                      {f.detected_char ? `（検出文字: ${f.detected_char}）` : ""}
                    </span>
                    <button
                      type="button"
                      className="skipped-file-rename"
                      disabled={busy}
                      onClick={() => startEdit(f)}
                    >
                      名前を変更
                    </button>
                  </>
                )}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
