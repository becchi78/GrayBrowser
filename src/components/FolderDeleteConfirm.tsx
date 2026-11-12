import type { DeleteTarget } from "../useWatchFolders";
import { ModalShell } from "./ModalShell";

interface Props {
  target: DeleteTarget;
  /** 解除中。ボタンを押せなくし、背景のクリックでも閉じない。 */
  deleting: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

// フォルダの登録解除の確認ダイアログ。
export function FolderDeleteConfirm({
  target,
  deleting,
  onCancel,
  onConfirm,
}: Props) {
  return (
    <ModalShell
      label="フォルダの登録を解除"
      className="folder-delete-confirm-dialog"
      testId="folder-delete-confirm-dialog"
      onOverlayClick={() => !deleting && onCancel()}
    >
      <p>
        このフォルダの登録を解除します。配下の{" "}
        <strong>{target.count}</strong>{" "}
        件の動画について、GrayBrowser上のタグ・レーティング・登録日の情報が失われます（動画ファイル自体は削除されません）。
      </p>
      <p className="folder-delete-confirm-path">{target.path}</p>
      <div className="dialog-footer">
        <button
          type="button"
          data-testid="folder-delete-cancel-btn"
          onClick={onCancel}
          disabled={deleting}
        >
          キャンセル
        </button>
        <button
          type="button"
          data-testid="folder-delete-confirm-btn"
          onClick={onConfirm}
          disabled={deleting}
        >
          {deleting ? "解除中..." : "登録を解除する"}
        </button>
      </div>
    </ModalShell>
  );
}
