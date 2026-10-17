import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

interface Props {
  open: boolean;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  /** 取り返しのつきにくい操作。確認ボタンを危険な見た目にする。 */
  danger?: boolean;
  /** 実行中。二重実行を防ぐため、確認ボタンを無効にし、閉じられなくする。 */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

// window.confirm は使わない。tauri-plugin-dialog が window.confirm を非同期
// 関数に差し替えるため Promise が返り、常に真として扱われて確認が効かない。
//
// 重複パネルは position の効く祖先を持つポップオーバー内にあり、そこに
// 描画すると position: fixed のオーバーレイが祖先に引っ張られるおそれが
// あるため、document.body へ portal で描画する。
export function ConfirmDialog({
  open,
  title,
  children,
  confirmLabel,
  danger = false,
  busy = false,
  onConfirm,
  onCancel,
}: Props) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  // 最初のフォーカスはキャンセルに置く。開いた直後に Enter を押しても
  // 破壊的な操作が実行されないようにするため。
  useEffect(() => {
    if (open) {
      cancelRef.current?.focus();
    }
  }, [open]);

  useEffect(() => {
    if (!open || busy) {
      return;
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, busy, onCancel]);

  if (!open) {
    return null;
  }

  return createPortal(
    <div className="dialog-overlay" onClick={() => !busy && onCancel()}>
      <div
        className="dialog confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        data-testid="confirm-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>{title}</h2>
        <div className="confirm-dialog-body">{children}</div>
        <div className="dialog-footer">
          <button
            type="button"
            ref={cancelRef}
            data-testid="confirm-dialog-cancel-btn"
            onClick={onCancel}
            disabled={busy}
          >
            キャンセル
          </button>
          <button
            type="button"
            className={danger ? "confirm-dialog-confirm confirm-dialog-confirm--danger" : undefined}
            data-testid="confirm-dialog-confirm-btn"
            onClick={onConfirm}
            disabled={busy}
          >
            {busy ? "処理中..." : confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
