import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import { assignTagToVideos, listAllTags } from "../api";
import { filterTagCandidates } from "../lib/bulkTag";
import type { TagDto } from "../types";

interface Props {
  /** 付与対象。null のとき閉じている。 */
  videoIds: string[] | null;
  /** 付与が成功したとき(ダイアログを閉じる前)に呼ばれる。一覧のタグ表示の更新用。 */
  onTagged: () => void;
  onClose: () => void;
}

// 成功したら結果を出さずに閉じる(件数の内訳は利用者に不要)。タグ名の正規化・検証は
// バックエンドだけが持ち、ここでは生の入力を渡して、返ったエラーを表示する。空入力は
// エラーにせず、OKボタンを押せなくして防ぐ。
export function BulkTagDialog({ videoIds, onTagged, onClose }: Props) {
  // 開くたびに本体を作り直すことで、入力・エラー・結果の状態を初期化する。
  if (videoIds === null) return null;
  return <BulkTagDialogBody videoIds={videoIds} onTagged={onTagged} onClose={onClose} />;
}

function BulkTagDialogBody({
  videoIds,
  onTagged,
  onClose,
}: {
  videoIds: string[];
  onTagged: () => void;
  onClose: () => void;
}) {
  const [input, setInput] = useState("");
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    listAllTags()
      .then((tags) => {
        if (!cancelled) setAllTags(tags);
      })
      .catch(console.error);
    inputRef.current?.focus();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (busy) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  const candidates = useMemo(() => filterTagCandidates(allTags, input), [allTags, input]);

  const canSubmit = input.trim() !== "";

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (busy || !canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await assignTagToVideos(videoIds, input);
      onTagged();
      onClose();
    } catch (err) {
      console.error(err);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  const title = `${videoIds.length} 件の動画にタグを付ける`;
  return createPortal(
    <div className="dialog-overlay" onClick={() => !busy && onClose()}>
      <div
        className="dialog bulk-tag-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        data-testid="bulk-tag-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>{title}</h2>
        <form onSubmit={handleSubmit}>
          <input
            ref={inputRef}
            type="text"
            className="bulk-tag-input"
            data-testid="bulk-tag-input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="タグ名(新しい名前も入力できます)"
            list="bulk-tag-suggestions"
            disabled={busy}
          />
          <datalist id="bulk-tag-suggestions">
            {allTags.map((tag) => (
              <option key={tag.id} value={tag.name} />
            ))}
          </datalist>
          {candidates.length > 0 && (
            <div className="bulk-tag-list" data-testid="bulk-tag-list">
              {candidates.map((tag) => (
                <button
                  key={tag.id}
                  type="button"
                  className="bulk-tag-candidate"
                  disabled={busy}
                  onClick={() => {
                    setInput(tag.name);
                    inputRef.current?.focus();
                  }}
                >
                  {tag.name}
                </button>
              ))}
            </div>
          )}
          {error && (
            <p className="bulk-tag-error" role="alert" data-testid="bulk-tag-error">
              {error}
            </p>
          )}
          <div className="dialog-footer">
            <button
              type="button"
              data-testid="bulk-tag-cancel-btn"
              onClick={onClose}
              disabled={busy}
            >
              キャンセル
            </button>
            <button type="submit" data-testid="bulk-tag-submit-btn" disabled={busy || !canSubmit}>
              {busy ? "処理中..." : "OK"}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body,
  );
}
