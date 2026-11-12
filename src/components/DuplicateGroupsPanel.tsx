import { ConfirmDialog } from "./ConfirmDialog";
import { DuplicateGroupItem } from "./DuplicateGroupItem";
import { useDuplicateGroups } from "../useDuplicateGroups";

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge, not this component's own
  // toggle button. The component stays mounted regardless of `open` so
  // the fetch effect and the onDedupUpdated subscription in
  // useDuplicateGroups keep the badge's count fresh in the background even
  // while the panel is closed.
  open: boolean;
  // Reports the current total member count up to StatusBar for the badge
  // label.
  onCountChange?: (count: number) => void;
}

export function DuplicateGroupsPanel({ refreshKey, open, onCountChange }: Props) {
  const {
    groups,
    checking,
    error,
    pending,
    busy,
    fileDeleteErrors,
    handleRefresh,
    runPendingAction,
    requestFileDelete,
    requestUnregister,
    cancelPending,
  } = useDuplicateGroups(refreshKey, open, onCountChange);

  if (!open) {
    return null;
  }

  return (
    <section className="duplicate-groups-panel status-panel" data-testid="status-panel">
      <div className="duplicate-groups-panel-row">
        <button type="button" onClick={handleRefresh} disabled={checking}>
          {checking ? "確認中..." : "重複を再チェック"}
        </button>
      </div>
      {error && <p className="duplicate-groups-error">{error}</p>}
      {groups.length === 0 ? (
        <p>重複候補は見つかっていません。</p>
      ) : (
        <ul className="duplicate-groups-list">
          {groups.map((group, index) => (
            <DuplicateGroupItem
              key={index}
              group={group}
              // 確認モーダルの表示中も行の操作を止める。背景へ Tab で移って別の
              // メンバーの操作を押すと、モーダルの対象が気づかないうちに
              // 切り替わってしまうため。
              busy={busy || pending !== null}
              fileDeleteErrors={fileDeleteErrors}
              onDelete={requestUnregister}
              onDeleteFile={requestFileDelete}
            />
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={pending !== null}
        title={pending?.kind === "file" ? "ファイルを削除" : "登録解除"}
        confirmLabel={pending?.kind === "file" ? "ゴミ箱に移動して削除" : "登録を解除する"}
        danger={pending?.kind === "file"}
        busy={busy}
        onConfirm={runPendingAction}
        onCancel={cancelPending}
      >
        {pending?.kind === "file" && (
          <>
            <p>
              この動画ファイルをゴミ箱に移動し、カタログからも削除します（ゴミ箱から元に戻せます）。
            </p>
            <p className="confirm-dialog-path">{pending.member.file_path}</p>
          </>
        )}
        {pending?.kind === "unregister" && (
          <>
            <p>このカタログ登録を解除します。</p>
            <p className="confirm-dialog-path">{pending.member.file_path}</p>
            <p>
              元の動画ファイルは削除されません。GrayBrowser上のタグ・レーティングなどカタログの情報は失われます。
            </p>
          </>
        )}
      </ConfirmDialog>
    </section>
  );
}
