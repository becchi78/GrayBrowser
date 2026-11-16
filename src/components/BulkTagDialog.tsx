import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { assignTagToVideos, listAllTags, listTagUsageForVideos, removeTagFromVideos } from "../api";
import { filterTagCandidates } from "../lib/bulkTag";
import type { TagDto, TagWithCountDto } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { ModalShell } from "./ModalShell";
import { errorText } from "../lib/errorText";

interface Props {
  /** 対象。null のとき閉じている。 */
  videoIds: string[] | null;
  /** 付与・解除が成功したとき(ダイアログを閉じる前)に呼ばれる。一覧のタグ表示の更新用。 */
  onTagged: () => void;
  onClose: () => void;
}

type Mode = "add" | "remove";

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
  const [mode, setMode] = useState<Mode>("add");
  const [input, setInput] = useState("");
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // 「外す」タブ。タブを開くたびに付いているタグを取り直す(取得に失敗したときの
  // やり直しも、タブを切り替えれば足りる)。
  const [usage, setUsage] = useState<TagWithCountDto[] | null>(null);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [selectedTagId, setSelectedTagId] = useState<number | null>(null);
  const [confirming, setConfirming] = useState(false);

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
    if (mode !== "remove") return;
    let cancelled = false;
    listTagUsageForVideos(videoIds)
      .then((rows) => {
        if (!cancelled) setUsage(rows);
      })
      .catch((err) => {
        console.error(err);
        if (!cancelled) setUsageError(errorText(err));
      });
    return () => {
      cancelled = true;
    };
  }, [mode, videoIds]);

  const candidates = useMemo(() => filterTagCandidates(allTags, input), [allTags, input]);

  const canSubmit = input.trim() !== "";
  const selectedTag = usage?.find((t) => t.id === selectedTagId) ?? null;

  function switchMode(next: Mode) {
    if (busy || next === mode) return;
    setMode(next);
    setError(null);
    setUsage(null);
    setUsageError(null);
    setSelectedTagId(null);
  }

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
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  async function confirmRemove() {
    if (busy || !selectedTag) return;
    setBusy(true);
    setError(null);
    try {
      await removeTagFromVideos(videoIds, selectedTag.id);
      setConfirming(false);
      onTagged();
      onClose();
    } catch (err) {
      console.error(err);
      setConfirming(false);
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  const title = `${videoIds.length} 件の動画にタグを付ける`;
  return (
    <>
      <ModalShell
        label={title}
        className="bulk-tag-dialog"
        testId="bulk-tag-dialog"
        onDismiss={onClose}
        dismissible={!busy && !confirming}
      >
        <h2>{title}</h2>
        <div className="bulk-tag-tabs" role="tablist" aria-label="操作">
          <button
            type="button"
            role="tab"
            id="bulk-tag-tab-add"
            aria-selected={mode === "add"}
            aria-controls="bulk-tag-panel-add"
            className="bulk-tag-tab"
            data-testid="bulk-tag-tab-add"
            disabled={busy}
            onClick={() => switchMode("add")}
          >
            付ける
          </button>
          <button
            type="button"
            role="tab"
            id="bulk-tag-tab-remove"
            aria-selected={mode === "remove"}
            aria-controls="bulk-tag-panel-remove"
            className="bulk-tag-tab"
            data-testid="bulk-tag-tab-remove"
            disabled={busy}
            onClick={() => switchMode("remove")}
          >
            外す
          </button>
        </div>
        {/* 入力の途中を失わないよう、「外す」に切り替えても付けるフォームは残して隠すだけにする。 */}
        <div
          role="tabpanel"
          id="bulk-tag-panel-add"
          aria-labelledby="bulk-tag-tab-add"
          hidden={mode !== "add"}
        >
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
            {mode === "add" && error && (
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
              <button
                type="submit"
                data-testid="bulk-tag-submit-btn"
                disabled={busy || !canSubmit}
              >
                {busy ? "処理中..." : "OK"}
              </button>
            </div>
          </form>
        </div>
        {mode === "remove" && (
          <div
            role="tabpanel"
            id="bulk-tag-panel-remove"
            aria-labelledby="bulk-tag-tab-remove"
            className="bulk-tag-remove"
          >
            {usageError ? (
              <p className="bulk-tag-error" role="alert" data-testid="bulk-tag-usage-error">
                {usageError}
              </p>
            ) : usage === null ? (
              <p data-testid="bulk-tag-usage-loading">読み込み中...</p>
            ) : usage.length === 0 ? (
              <p data-testid="bulk-tag-usage-empty">選択した動画にタグは付いていません。</p>
            ) : (
              <div
                className="bulk-tag-usage"
                role="radiogroup"
                aria-label="外すタグ"
                data-testid="bulk-tag-usage-list"
              >
                {usage.map((tag) => (
                  <label key={tag.id} className="bulk-tag-usage-item">
                    <input
                      type="radio"
                      name="bulk-tag-remove-target"
                      checked={tag.id === selectedTagId}
                      disabled={busy}
                      onChange={() => setSelectedTagId(tag.id)}
                    />
                    <span>
                      {tag.name}({tag.video_count}/{videoIds.length} 件)
                    </span>
                  </label>
                ))}
              </div>
            )}
            {error && (
              <p className="bulk-tag-error" role="alert" data-testid="bulk-tag-remove-error">
                {error}
              </p>
            )}
            <div className="dialog-footer">
              <button
                type="button"
                data-testid="bulk-tag-remove-cancel-btn"
                onClick={onClose}
                disabled={busy}
              >
                キャンセル
              </button>
              <button
                type="button"
                data-testid="bulk-tag-remove-btn"
                disabled={busy || selectedTag === null}
                onClick={() => setConfirming(true)}
              >
                外す
              </button>
            </div>
          </div>
        )}
      </ModalShell>

      {/* A sibling of the ModalShell, not a child; see ModalShell for why. */}
      <ConfirmDialog
        open={confirming && selectedTag !== null}
        title="タグを外す"
        confirmLabel="外す"
        danger
        busy={busy}
        onConfirm={confirmRemove}
        onCancel={() => setConfirming(false)}
      >
        {selectedTag && (
          <p>{`『${selectedTag.name}』を ${selectedTag.video_count} 件の動画から外します。元に戻せません。`}</p>
        )}
      </ConfirmDialog>
    </>
  );
}
