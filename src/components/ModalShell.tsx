import type { FormEvent, MouseEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import { useEscapeKey } from "../useEscapeKey";

interface Props {
  /** `.dialog` の aria-label。 */
  label: string;
  /** `dialog` に続けて付ける class。 */
  className: string;
  testId: string;
  /** 背景のクリックと Escape で呼ぶ。undefined ならどちらでも何もしない。 */
  onDismiss?: () => void;
  /** false の間は、背景のクリックでも Escape でも onDismiss を呼ばない。既定は true。 */
  dismissible?: boolean;
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
// 背景のクリックで閉じられるときだけ Escape でも閉じる。この規則を各ダイアログで
// 守らせるため、両方を onDismiss と dismissible の1組で受け、ここで同じ条件で扱う。
// 閉じてはいけない間(処理中や、子のダイアログが開いている間)は dismissible を
// false にする。false の間は Escape のリスナーを登録しないので、子が開いている
// 間の Escape は子だけに届く。子が閉じた後の再描画で親のリスナーが登録されるため、
// 子を閉じた同じ keydown で親まで閉じることはない(dispatch 中に足したリスナーは、
// その event では呼ばれない)。
//
// 子のダイアログは、親の ModalShell の外に兄弟として描くこと。React のイベントは
// portal を越えて React の木に沿って伝わるので、子を親の中に描くと、子のイベントの
// 経路に親の handler(背景のクリック、Settings のような form の submit など)が入る。
// 子を親の背景の直下(`.dialog` の外)に置くと、子の背景のクリックが親の背景に届いて
// 親まで閉じる。`.dialog` の中なら stopPropagation でクリックは止まるが、それに頼らない。
export function ModalShell({
  label,
  className,
  testId,
  onDismiss,
  dismissible = true,
  onSubmit,
  children,
}: Props) {
  const dismiss = dismissible ? onDismiss : undefined;
  useEscapeKey(() => dismiss?.(), dismiss !== undefined);

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
    <div className="dialog-overlay" onClick={dismiss && (() => dismiss())}>
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
