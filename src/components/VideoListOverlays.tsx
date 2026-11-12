import { BulkTagDialog } from "./BulkTagDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { ContextMenu } from "./ContextMenu";

/** 右クリックで開くメニューの位置(viewport 座標)と、操作の対象にする動画。 */
export interface VideoListMenu {
  x: number;
  y: number;
  ids: string[];
}

interface Props {
  /** null のときメニューは閉じている。 */
  menu: VideoListMenu | null;
  onCloseMenu: () => void;
  /** メニューの「タグを設定...」。対象はメニューの ids。 */
  onRequestBulkTag: (ids: string[]) => void;
  /** 一括付与の対象。null のときダイアログは閉じている。 */
  bulkTagIds: string[] | null;
  onTagged: () => void;
  onCloseBulkTag: () => void;
  /** メニューの「登録から削除」。対象はメニューの ids。 */
  onRequestUnregister: (ids: string[]) => void;
  /** 登録から削除する対象。null のとき確認は閉じている。 */
  pendingUnregister: string[] | null;
  unregisterBusy: boolean;
  unregisterError: string | null;
  onConfirmUnregister: () => void;
  onCancelUnregister: () => void;
}

// 一覧の上に重ねて出す、行の右クリックメニュー・一括付与のダイアログ・
// 登録から削除の確認。
export function VideoListOverlays({
  menu,
  onCloseMenu,
  onRequestBulkTag,
  bulkTagIds,
  onTagged,
  onCloseBulkTag,
  onRequestUnregister,
  pendingUnregister,
  unregisterBusy,
  unregisterError,
  onConfirmUnregister,
  onCancelUnregister,
}: Props) {
  return (
    <>
      {menu && (
        <ContextMenu
          position={{ x: menu.x, y: menu.y }}
          items={[
            {
              label: menu.ids.length >= 2 ? `タグを設定...(${menu.ids.length} 件)` : "タグを設定...",
              onSelect: () => onRequestBulkTag(menu.ids),
            },
            {
              label: menu.ids.length >= 2 ? `登録から削除(${menu.ids.length} 件)` : "登録から削除",
              danger: true,
              onSelect: () => onRequestUnregister(menu.ids),
            },
          ]}
          onClose={onCloseMenu}
        />
      )}
      <BulkTagDialog videoIds={bulkTagIds} onTagged={onTagged} onClose={onCloseBulkTag} />
      <ConfirmDialog
        open={pendingUnregister !== null}
        title="登録から削除"
        confirmLabel="登録から削除する"
        danger
        busy={unregisterBusy}
        onConfirm={onConfirmUnregister}
        onCancel={onCancelUnregister}
      >
        <p>{pendingUnregister?.length ?? 0} 件の動画を登録から削除します。</p>
        <p>
          元の動画ファイルは削除されません。登録フォルダが監視・再スキャンの対象のままだと、次の更新チェックで再登録される場合があります。
        </p>
        {unregisterError && (
          <p className="confirm-dialog-error" role="alert" data-testid="confirm-dialog-error">
            {unregisterError}
          </p>
        )}
      </ConfirmDialog>
    </>
  );
}
