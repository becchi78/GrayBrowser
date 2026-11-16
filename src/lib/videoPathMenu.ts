import type { ContextMenuItem } from "../components/ContextMenu";

/** パスの操作の対象 1 件。offline は「オフライン、または保存先フォルダに今届かない」。 */
export interface PathMenuTarget {
  id: string;
  file_path: string;
  offline: boolean;
}

export interface PathMenuActions {
  onCopy: (paths: string[]) => void;
  onReveal: (id: string) => void;
}

/** 所属フォルダを解決できずに `::ラベル\…` の形で持っているパスか。 */
export function isUnresolvedPath(path: string): boolean {
  return path.startsWith("::");
}

/** 「エクスプローラーで開く」を出せるか。1 件のオンラインで、パスを解決できている場合だけ。 */
export function canReveal(targets: readonly PathMenuTarget[]): boolean {
  if (targets.length !== 1) return false;
  const [target] = targets;
  return !isUnresolvedPath(target.file_path) && !target.offline;
}

/**
 * 右クリックメニューの「パスをコピー」「エクスプローラーで開く」。一覧と重複
 * パネルで共通。コピーはオフラインでも使えるよう常に出す。開けない対象では
 * 「エクスプローラーで開く」を出さない。
 */
export function buildPathMenuItems(
  targets: readonly PathMenuTarget[],
  actions: PathMenuActions,
): ContextMenuItem[] {
  if (targets.length === 0) return [];
  const items: ContextMenuItem[] = [
    {
      label: targets.length >= 2 ? `パスをコピー(${targets.length} 件)` : "パスをコピー",
      onSelect: () => actions.onCopy(targets.map((t) => t.file_path)),
    },
  ];
  if (canReveal(targets)) {
    items.push({
      label: "エクスプローラーで開く",
      onSelect: () => actions.onReveal(targets[0].id),
    });
  }
  return items;
}
