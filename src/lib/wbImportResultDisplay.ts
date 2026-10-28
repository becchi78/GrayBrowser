// .wbインポート結果画面（WbImportDialog.tsx）の表示内容を決める純関数。
// 何を主表示にし、何を「要対応」として目立たせ、何を折りたたむかの判断を
// コンポーネントから切り出し、node --experimental-strip-types --test で
// 単体テストできるようにする。集計ロジック・件数の定義はここでは変えず、
// WbImportSummaryの値を並べ替えて文言にするだけ。
//
// 主表示は「.wb内の動画 = インポート(オンライン+オフライン) + スキップ +
// 未登録 + 取り込めなかった行」で突き合わせられるように並べる。各行は
// ちょうど1つの区分に入るので、画面の数字を足すと必ず合計に一致する。
//
// 振り分けの基準:
//  - 要対応: 利用者が何かしないと状態が良くならない/確認すべきもの。
//    自動対応できなかった旧サムネイル、旧サムネイル変換の失敗、機種依存文字の
//    ため未登録、取り込めなかった行、タグ付与の amber/failed 判定。該当が
//    あるときだけ出す。
//  - 内訳: 対応不要の情報。0件の項目は出さない。
// オンライン動画の旧サムネイルは、取り込めなかったと誤解されやすいため、
// 件数も注記も出さない（動画本体から自動生成される）。
import type { WbImportSummary } from "../types";
import { classifyTagsAssigned } from "./wbImportTagsDisplay.ts";

export type AttentionTone = "amber" | "failed";

export interface AttentionItem {
  key: "unmatched" | "failed" | "invalid-name" | "failed-rows" | "tags";
  tone: AttentionTone;
  text: string;
  /** 「確認する」ボタンで補足を開ける項目。 */
  expandable: boolean;
}

export interface DetailItem {
  key: "tags" | "clamped";
  text: string;
}

export interface WbImportResultDisplay {
  /** .wb内の動画の総数。 */
  totalText: string;
  /** インポートした件数と、オンライン/オフラインの内訳。 */
  registeredText: string;
  /** オフラインが1件以上のときだけ。旧サムネイルが紐付いた件数を、オフライン件数に対して示す。 */
  offlineThumbnailsText: string | null;
  /** 監視フォルダを追加しなかったときは null（0件は出さない）。 */
  watchFoldersAddedText: string | null;
  /** 登録済みのためスキップした行がないときは null（0件は出さない）。 */
  skippedText: string | null;
  attention: AttentionItem[];
  details: DetailItem[];
  /** 内訳に並べる、追加した監視フォルダのパス。 */
  watchFolders: string[];
}

export function buildWbImportResultDisplay(s: WbImportSummary): WbImportResultDisplay {
  const attention: AttentionItem[] = [];
  if (s.thumbnails_unmatched > 0) {
    attention.push({
      key: "unmatched",
      tone: "amber",
      text: `自動対応できなかった旧サムネイル: ${s.thumbnails_unmatched}件 — エラーではありません。手動で紐付けてください。`,
      expandable: true,
    });
  }
  if (s.thumbnails_failed > 0) {
    attention.push({
      key: "failed",
      tone: "failed",
      text: `旧サムネイル変換に失敗: ${s.thumbnails_failed}件`,
      expandable: false,
    });
  }
  if (s.skipped_invalid_name > 0) {
    attention.push({
      key: "invalid-name",
      tone: "amber",
      text: `機種依存文字のため未登録: ${s.skipped_invalid_name}件 — 「未登録」パネルで名前を変更すると登録できます`,
      expandable: false,
    });
  }
  if (s.failed_rows > 0) {
    attention.push({
      key: "failed-rows",
      tone: "failed",
      text: `取り込めなかった行: ${s.failed_rows}件`,
      expandable: false,
    });
  }

  const tags = classifyTagsAssigned(s);
  const details: DetailItem[] = [];
  if (tags.tone === "amber" || tags.tone === "failed") {
    attention.push({ key: "tags", tone: tags.tone, text: tags.text, expandable: false });
  } else if (s.tags_assigned > 0) {
    // "muted" は原因つきの0件表示なので、件数0の項目として出さない。
    details.push({ key: "tags", text: tags.text });
  }
  if (s.clamped_scores > 0) {
    details.push({ key: "clamped", text: `score丸め: ${s.clamped_scores}件` });
  }

  return {
    totalText: `.wb内の動画: ${s.total}件`,
    registeredText: `インポート: ${s.registered}件（オンライン ${s.registered_online}件 / オフライン ${s.registered_offline}件）`,
    offlineThumbnailsText:
      s.registered_offline > 0
        ? `オフライン動画の旧サムネイル紐付け成功: ${s.thumbnails_linked}件（${s.registered_offline}件中）`
        : null,
    watchFoldersAddedText:
      s.watch_folders_added.length > 0
        ? `監視フォルダに追加: ${s.watch_folders_added.length}件`
        : null,
    skippedText: s.skipped > 0 ? `登録済みのためスキップ: ${s.skipped}件` : null,
    attention,
    details,
    watchFolders: s.watch_folders_added,
  };
}
