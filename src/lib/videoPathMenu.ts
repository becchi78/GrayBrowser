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

/** 「エクスプローラーで開く」を押せない理由。押せるなら null。 */
export function revealDisabledReason(targets: readonly PathMenuTarget[]): string | null {
  if (targets.length >= 2) return "複数選択のため開けません";
  const [target] = targets;
  if (!target) return "対象がありません";
  if (isUnresolvedPath(target.file_path)) return "パスを解決できないため開けません";
  if (target.offline) return "オフラインのため開けません";
  return null;
}

/**
 * 右クリックメニューの「パスをコピー」「エクスプローラーで開く」。一覧と重複
 * パネルで共通。コピーはオフラインでも使えるよう常に有効にする。
 */
export function buildPathMenuItems(
  targets: readonly PathMenuTarget[],
  actions: PathMenuActions,
): ContextMenuItem[] {
  if (targets.length === 0) return [];
  const reason = revealDisabledReason(targets);
  return [
    {
      label: targets.length >= 2 ? `パスをコピー(${targets.length} 件)` : "パスをコピー",
      onSelect: () => actions.onCopy(targets.map((t) => t.file_path)),
    },
    {
      label: reason ? `エクスプローラーで開く(${reason})` : "エクスプローラーで開く",
      disabled: reason !== null,
      onSelect: () => actions.onReveal(targets[0].id),
    },
  ];
}
