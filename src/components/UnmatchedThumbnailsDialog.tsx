import { useEffect, useRef, useState } from "react";
import {
  dismissUnmatchedThumbnail,
  linkUnmatchedThumbnail,
  listUnmatchedThumbnails,
  searchLinkCandidates,
} from "../api";
import { abbreviatePath } from "../lib/abbreviatePath";
import { createLatestOnlyGuard } from "../lib/latestRequest";
import {
  filterUnmatchedByKind,
  overwriteNotice,
  unmatchedCountText,
  unmatchedKindLabel,
  unmatchedRowKey,
} from "../lib/unmatchedThumbnails";
import type { LinkCandidateDto, UnmatchedThumbnailDto, UnmatchedThumbnailKind } from "../types";

interface Props {
  onClose: () => void;
  /** 動画のサムネイルが置き換わった(または置き換えに失敗して消えた)ときに呼ぶ。 */
  onThumbnailsReplaced: (videoId: string) => void;
  /** 指定すると、その原因の行だけを並べる(見出しの件数も同じ範囲)。省略すると全件。 */
  kind?: UnmatchedThumbnailKind;
}

const PATH_DISPLAY_MAX_CHARS = 60;
const CANDIDATE_LIMIT = 50;
const SEARCH_DEBOUNCE_MS = 300;

interface PickerProps {
  row: UnmatchedThumbnailDto;
  busy: boolean;
  onLink: (videoId: string) => void;
}

// 行を開いている間だけマウントされる。検索語と選択は行を閉じると破棄される。
function LinkPicker({ row, busy, onLink }: PickerProps) {
  const [term, setTerm] = useState(row.default_search_term ?? "");
  const [candidates, setCandidates] = useState<LinkCandidateDto[] | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [selected, setSelected] = useState<LinkCandidateDto | null>(null);
  const guardRef = useRef(createLatestOnlyGuard());

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      // 入力が続くと古い要求の応答が後から届きうるので、最新の要求だけ反映する。
      const requestId = guardRef.current.begin();
      searchLinkCandidates(term, CANDIDATE_LIMIT)
        .then((found) => {
          if (cancelled || !guardRef.current.isLatest(requestId)) return;
          setCandidates(found);
          setSearchError(null);
          // 結果から消えた候補を選んだままにしない。
          setSelected((prev) => (prev && found.some((c) => c.id === prev.id) ? prev : null));
        })
        .catch((e) => {
          if (cancelled || !guardRef.current.isLatest(requestId)) return;
          console.error(e);
          setSearchError(String(e));
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [term]);

  const notice = selected ? overwriteNotice(selected.thumbnail_ready) : null;

  return (
    <div className="unmatched-picker" data-testid="unmatched-picker">
      <input
        type="text"
        className="unmatched-picker-search"
        data-testid="unmatched-picker-search"
        placeholder="動画のファイル名で検索"
        value={term}
        onChange={(e) => setTerm(e.target.value)}
        disabled={busy}
      />
      {searchError && (
        <p className="wb-import-error" role="alert">
          {searchError}
        </p>
      )}
      {candidates !== null && candidates.length === 0 && !searchError && (
        <p className="wb-import-result-detail" data-testid="unmatched-picker-empty">
          紐付けできるオフラインの動画が見つかりません。
        </p>
      )}
      {candidates !== null && candidates.length > 0 && (
        <ul className="unmatched-candidates" data-testid="unmatched-candidates">
          {candidates.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                className={
                  "unmatched-candidate" + (selected?.id === c.id ? " unmatched-candidate--selected" : "")
                }
                data-testid="unmatched-candidate"
                title={c.file_path}
                aria-pressed={selected?.id === c.id}
                disabled={busy}
                onClick={() => setSelected(c)}
              >
                <span className="unmatched-candidate-name">{c.file_name}</span>
                {c.thumbnail_ready && (
                  <span className="unmatched-badge" data-testid="unmatched-candidate-ready">
                    紐付け済み
                  </span>
                )}
                <span className="unmatched-candidate-path">
                  {abbreviatePath(c.file_path, PATH_DISPLAY_MAX_CHARS)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {selected && (
        <div className="unmatched-confirm" data-testid="unmatched-confirm">
          <p className="wb-import-result-line">
            「{selected.file_name}」に、この旧サムネイルを紐付けます。
          </p>
          {notice && (
            <p className="wb-import-result-line unmatched-overwrite" data-testid="unmatched-overwrite">
              {notice}
            </p>
          )}
          <button
            type="button"
            data-testid="unmatched-link-btn"
            disabled={busy}
            onClick={() => onLink(selected.id)}
          >
            {busy ? "紐付け中..." : "この動画に紐付ける"}
          </button>
        </div>
      )}
    </div>
  );
}

// WbImportDialog が開いている間だけマウントする。閉じると一覧の取得結果や
// 行ごとのエラーは破棄され、開き直すたびに保存済みの一覧を読み直す。
export function UnmatchedThumbnailsDialog({ onClose, onThumbnailsReplaced, kind }: Props) {
  const [allRows, setRows] = useState<UnmatchedThumbnailDto[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  // state の更新を待たず二重クリックを弾く。紐付けは ffmpeg を走らせるので数秒かかり、
  // その間に別の操作が走ると同じ動画のスロットファイルを取り合う。
  const inFlightRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    listUnmatchedThumbnails()
      .then((list) => {
        if (!cancelled) setRows(list);
      })
      .catch((e) => {
        if (cancelled) return;
        console.error(e);
        setLoadError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  function withoutError(errors: Record<string, string>, key: string) {
    const next = { ...errors };
    delete next[key];
    return next;
  }

  function removeRow(key: string) {
    setRows((prev) => prev && prev.filter((r) => unmatchedRowKey(r) !== key));
    setOpenKey((prev) => (prev === key ? null : prev));
    setRowErrors((prev) => withoutError(prev, key));
  }

  async function runRowAction(row: UnmatchedThumbnailDto, action: () => Promise<void>) {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    const key = unmatchedRowKey(row);
    setBusyKey(key);
    setRowErrors((prev) => withoutError(prev, key));
    try {
      await action();
      removeRow(key);
    } catch (e) {
      console.error(e);
      setRowErrors((prev) => ({ ...prev, [key]: String(e) }));
    } finally {
      inFlightRef.current = false;
      setBusyKey(null);
    }
  }

  function handleLink(row: UnmatchedThumbnailDto, videoId: string) {
    return runRowAction(row, async () => {
      try {
        await linkUnmatchedThumbnail(row.folder_path, row.file_name, videoId);
      } finally {
        // 上書きの途中で失敗すると、元のサムネイルも消えていることがある。
        // 成否にかかわらず、その動画の表示用キャッシュを捨てさせる。
        onThumbnailsReplaced(videoId);
      }
    });
  }

  function handleDismiss(row: UnmatchedThumbnailDto) {
    return runRowAction(row, () => dismissUnmatchedThumbnail(row.folder_path, row.file_name));
  }

  const anyBusy = busyKey !== null;
  const rows = allRows === null ? null : filterUnmatchedByKind(allRows, kind);

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div
        className="dialog unmatched-dialog"
        data-testid="unmatched-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>自動対応できなかった旧サムネイル</h2>
        {rows !== null && rows.length > 0 && (
          <p className="wb-import-result-detail" data-testid="unmatched-count">
            {unmatchedCountText(rows.length)}
          </p>
        )}

        {loadError && (
          <p className="wb-import-error" role="alert">
            一覧を読み込めませんでした: {loadError}
          </p>
        )}
        {rows === null && !loadError && <p className="wb-import-result-detail">読み込み中...</p>}
        {rows !== null && rows.length === 0 && (
          <p className="wb-import-result-line" data-testid="unmatched-empty">
            自動対応できなかった旧サムネイルはありません。
          </p>
        )}

        {rows !== null && rows.length > 0 && (
          <ul className="unmatched-list" data-testid="unmatched-list">
            {rows.map((row) => {
              const key = unmatchedRowKey(row);
              const isOpen = openKey === key;
              const error = rowErrors[key];
              return (
                <li key={key} className="unmatched-row" data-testid="unmatched-row">
                  <div className="unmatched-row-head">
                    <div className="unmatched-row-info">
                      <span className="unmatched-row-name" title={row.file_name}>
                        {row.file_name}
                      </span>
                      <span className="unmatched-kind" data-testid="unmatched-kind">
                        {unmatchedKindLabel(row.kind)}
                      </span>
                      <span className="unmatched-row-folder" title={row.folder_path}>
                        {abbreviatePath(row.folder_path, PATH_DISPLAY_MAX_CHARS)}
                      </span>
                    </div>
                    <div className="unmatched-row-actions">
                      <button
                        type="button"
                        data-testid="unmatched-pick-btn"
                        aria-expanded={isOpen}
                        disabled={anyBusy}
                        onClick={() => setOpenKey(isOpen ? null : key)}
                      >
                        {isOpen ? "閉じる" : "動画を選ぶ"}
                      </button>
                      <button
                        type="button"
                        data-testid="unmatched-dismiss-btn"
                        disabled={anyBusy}
                        onClick={() => handleDismiss(row)}
                      >
                        一覧から外す
                      </button>
                    </div>
                  </div>
                  {error && (
                    <p className="wb-import-error" role="alert" data-testid="unmatched-row-error">
                      {error}
                    </p>
                  )}
                  {isOpen && (
                    <LinkPicker
                      row={row}
                      busy={busyKey === key}
                      onLink={(videoId) => handleLink(row, videoId)}
                    />
                  )}
                </li>
              );
            })}
          </ul>
        )}

        <div className="dialog-footer">
          <button type="button" data-testid="unmatched-close-btn" onClick={onClose}>
            閉じる
          </button>
        </div>
      </div>
    </div>
  );
}
