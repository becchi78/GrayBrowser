import { ConfirmDialog } from "./ConfirmDialog";
import type { StatusPanelProps } from "./StatusPanelProps";
import { DuplicateGroupItem } from "./DuplicateGroupItem";
import { useDuplicateGroups } from "../useDuplicateGroups";

export function DuplicateGroupsPanel({ refreshKey, open, onCountChange }: StatusPanelProps) {
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
        danger
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
