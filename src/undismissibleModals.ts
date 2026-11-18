// 閉じられない(dismissible=false)ModalShell の数。処理中のダイアログを、メニューからの
// 別ダイアログの要求で差し替えてしまわないよう、開く側が問い合わせる。
// ModalShell が effect で増減するので、描画中の値ではなくコミット済みの状態を反映する。
let count = 0;

export function registerUndismissibleModal(): () => void {
  count += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    count -= 1;
  };
}

export function hasUndismissibleModal(): boolean {
  return count > 0;
}
