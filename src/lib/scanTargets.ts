// FolderDialog の「再スキャン」ボタン用の、JSX非依存の純粋関数。
// 手動スキャンの対象はバックエンド(`start_scan`)が「更新チェックが入っている
// フォルダ」に絞るので、画面側も同じ数え方でボタンの有効/無効を決める。

/** 再スキャンの対象になるフォルダ数(更新チェックが入っているフォルダ)。 */
export function countScanTargets(folders: readonly string[], checkDisabled: readonly string[]): number {
  return folders.filter((f) => !checkDisabled.includes(f)).length;
}

export type RescanState =
  | { enabled: true }
  | { enabled: false; reason: "no-folders" | "all-check-off" };

/** 再スキャンボタンの状態。無効のときは、その理由を表示できるよう種別を返す。 */
export function rescanState(folders: readonly string[], checkDisabled: readonly string[]): RescanState {
  if (folders.length === 0) {
    return { enabled: false, reason: "no-folders" };
  }
  if (countScanTargets(folders, checkDisabled) === 0) {
    return { enabled: false, reason: "all-check-off" };
  }
  return { enabled: true };
}
