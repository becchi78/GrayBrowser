// 「自動対応できなかった旧サムネイル」一覧(UnmatchedThumbnailsDialog.tsx)の
// 表示用の純関数。node --experimental-strip-types --test で単体テストする。
import type { UnmatchedThumbnailDto, UnmatchedThumbnailKind } from "../types";

export function unmatchedKindLabel(kind: UnmatchedThumbnailKind): string {
  return kind === "empty_hash" ? "空のhash" : "対応する動画なし";
}

/** 0件のときは null(何も出さない)。 */
export function unmatchedCountText(count: number): string | null {
  return count > 0 ? `自動対応できなかった旧サムネイル: ${count}件` : null;
}

/** フォルダ違いの同名ファイルを区別するための行の識別子。 */
export function unmatchedRowKey(row: Pick<UnmatchedThumbnailDto, "folder_path" | "file_name">) {
  return JSON.stringify([row.folder_path, row.file_name]);
}

/** 既に付いているサムネイルを置き換えることを、確認の前に明示するための注意文。 */
export function overwriteNotice(thumbnailReady: boolean): string | null {
  return thumbnailReady
    ? "この動画には既にサムネイルがあります。紐付けると置き換えられ、失敗した場合は今のサムネイルも消えます。"
    : null;
}
