import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  assignTagsToVideos,
  listAllTags,
  listTagUsageForVideos,
  removeTagsFromVideos,
} from "../api";
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

// 確認文にタグ名をそのまま並べる上限。これを超えると件数にして、文が長くなりすぎないようにする。
const MAX_NAMES_IN_CONFIRM = 3;

interface ToggleItem {
  id: number;
  label: string;
}

// 複数選択のタグ一覧。選択状態はボタンの aria-pressed で伝える(押すたびに切り替わる)。
function TagToggleList({
  items,
  selected,
  disabled,
  ariaLabel,
  testId,
  onToggle,
}: {
  items: ToggleItem[];
  selected: ReadonlySet<number>;
  disabled: boolean;
  ariaLabel: string;
  testId: string;
  onToggle: (id: number) => void;
}) {
  return (
    <div className="bulk-tag-list" role="group" aria-label={ariaLabel} data-testid={testId}>
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          className="bulk-tag-candidate"
          aria-pressed={selected.has(item.id)}
          disabled={disabled}
          onClick={() => onToggle(item.id)}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

function toggled(set: ReadonlySet<number>, id: number): Set<number> {
  const next = new Set(set);
  if (!next.delete(id)) next.add(id);
  return next;
}

function removeConfirmText(names: string[]): string {
  const target =
    names.length <= MAX_NAMES_IN_CONFIRM
      ? names.map((n) => `『${n}』`).join("")
      : `選んだ ${names.length} 個のタグ`;
  return `${target}を、付いている動画から外します。元に戻せません。`;
}

// 成功したら結果を出さずに閉じる(件数の内訳は利用者に不要)。タグ名の正規化・検証は
// バックエンドだけが持ち、ここでは生の入力を渡して、返ったエラーを表示する。何も
// 選ばず入力も空のときはエラーにせず、OKボタンを押せなくして防ぐ。
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
  // 入力欄は新しいタグ名だけを受け持ち、既存タグは一覧で選ぶ。
  const [input, setInput] = useState("");
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [addSelected, setAddSelected] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // 「外す」タブ。タブを開くたびに付いているタグを取り直す(取得に失敗したときの
  // やり直しも、タブを切り替えれば足りる)。
  const [usage, setUsage] = useState<TagWithCountDto[] | null>(null);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [removeSelected, setRemoveSelected] = useState<ReadonlySet<number>>(new Set());
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

  const canSubmit = addSelected.size > 0 || input.trim() !== "";
  const selectedUsage = usage?.filter((t) => removeSelected.has(t.id)) ?? [];

  function switchMode(next: Mode) {
    if (busy || next === mode) return;
    setMode(next);
    setError(null);
    setUsage(null);
    setUsageError(null);
    setRemoveSelected(new Set());
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (busy || !canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      // 一覧で選んだ既存タグ(一覧の並び順)に、入力欄の新しい名前を足す。選んだタグと
      // 同じ名前を入力した場合の重複はバックエンドが取り除く。
      const names = allTags.filter((t) => addSelected.has(t.id)).map((t) => t.name);
      if (input.trim() !== "") names.push(input);
      await assignTagsToVideos(videoIds, names);
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
    if (busy || selectedUsage.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await removeTagsFromVideos(
        videoIds,
        selectedUsage.map((t) => t.id),
      );
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
              placeholder="タグ名"
              aria-label="タグ名"
              disabled={busy}
            />
            {allTags.length > 0 && (
              <TagToggleList
                items={allTags.map((t) => ({ id: t.id, label: t.name }))}
                selected={addSelected}
                disabled={busy}
                ariaLabel="付ける既存のタグ"
                testId="bulk-tag-list"
                onToggle={(id) => setAddSelected((prev) => toggled(prev, id))}
              />
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
              <TagToggleList
                items={usage.map((t) => ({
                  id: t.id,
                  label: `${t.name}(${t.video_count}/${videoIds.length} 件)`,
                }))}
                selected={removeSelected}
                disabled={busy}
                ariaLabel="外すタグ"
                testId="bulk-tag-usage-list"
                onToggle={(id) => setRemoveSelected((prev) => toggled(prev, id))}
              />
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
                disabled={busy || selectedUsage.length === 0}
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
        open={confirming && selectedUsage.length > 0}
        title="タグを外す"
        confirmLabel="外す"
        danger
        busy={busy}
        onConfirm={confirmRemove}
        onCancel={() => setConfirming(false)}
      >
        {selectedUsage.length > 0 && <p>{removeConfirmText(selectedUsage.map((t) => t.name))}</p>}
      </ConfirmDialog>
    </>
  );
}
