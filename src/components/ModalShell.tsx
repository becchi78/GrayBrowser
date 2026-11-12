import type { FormEvent, MouseEvent, ReactNode } from "react";
import { createPortal } from "react-dom";

interface Props {
  /** `.dialog` の aria-label。 */
  label: string;
  /** `dialog` に続けて付ける class。 */
  className: string;
  testId: string;
  /** 背景のクリックで呼ぶ。undefined なら背景をクリックしても何もしない。 */
  onOverlayClick?: () => void;
  /** 指定すると `.dialog` を form(noValidate)として描き、送信をこれに渡す。 */
  onSubmit?: (e: FormEvent<HTMLFormElement>) => void;
  children: ReactNode;
}

// ダイアログの外側(背景と `.dialog` の要素)。
//
// ダイアログは、ステータスバーのパネル(重複・生成失敗)のように position の効く
// 祖先を持つポップオーバーの中からも開かれる。そこに描画すると position: fixed の
// オーバーレイが祖先に引っ張られるおそれがあるため、どのダイアログも document.body へ
// portal で描画する。
//
// 子のダイアログは、親の ModalShell の外に兄弟として描くこと。React のイベントは
// portal を越えて React の木に沿って伝わるので、子を親の中に描くと、子のイベントの
// 経路に親の handler(背景のクリック、Settings のような form の submit など)が入る。
// 子を親の背景の直下(`.dialog` の外)に置くと、子の背景のクリックが親の背景に届いて
// 親まで閉じる。`.dialog` の中なら stopPropagation でクリックは止まるが、それに頼らない。
export function ModalShell({ label, className, testId, onOverlayClick, onSubmit, children }: Props) {
  const dialogProps = {
    className: `dialog ${className}`,
    role: "dialog",
    "aria-modal": "true",
    "aria-label": label,
    "data-testid": testId,
    // 中のクリックが背景に届いて閉じないようにする。
    onClick: (e: MouseEvent) => e.stopPropagation(),
  } as const;

  return createPortal(
    <div className="dialog-overlay" onClick={onOverlayClick}>
      {onSubmit ? (
        <form {...dialogProps} onSubmit={onSubmit} noValidate>
          {children}
        </form>
      ) : (
        <div {...dialogProps}>{children}</div>
      )}
    </div>,
    document.body,
  );
}
