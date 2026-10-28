// .wbインポート結果画面（WbImportDialog.tsx）の表示内容を決める純関数。
// 何を主表示にし、何を「要対応」として目立たせ、何を折りたたむかの判断を
// コンポーネントから切り出し、node --experimental-strip-types --test で
// 単体テストできるようにする。集計ロジック・件数の定義はここでは変えず、
// WbImportSummaryの値を並べ替えて文言にするだけ。
//
// 振り分けの基準:
//  - 要対応: 利用者が何かしないと状態が良くならない/確認すべきもの。
//    自動対応できなかった旧サムネイル、旧サムネイル変換の失敗、機種依存文字の
//    ため未登録、タグ付与の amber/failed 判定。該当があるときだけ出す。
//  - 内訳: 対応不要の情報。0件の項目は出さない。
import type { WbImportSummary } from "../types";
import { classifyTagsAssigned } from "./wbImportTagsDisplay.ts";

export type AttentionTone = "amber" | "failed";

export interface AttentionItem {
  key: "unmatched" | "failed" | "invalid-name" | "tags";
  tone: AttentionTone;
  text: string;
  /** 「確認する」ボタンで補足を開ける項目。 */
  expandable: boolean;
}

export interface DetailItem {
  key: "tags" | "star-ratings" | "skipped-online" | "linked" | "skipped" | "clamped";
  text: string;
}

export interface WbImportResultDisplay {
  registeredText: string;
  /** 監視フォルダを追加しなかったときは null（0件は出さない）。 */
  watchFoldersAddedText: string | null;
  attention: AttentionItem[];
  details: DetailItem[];
  /** 内訳に並べる、追加した監視フォルダのパス。 */
  watchFolders: string[];
  /** オンライン動画の旧サムネイルがあるときだけ、自動生成の注記を出す。 */
  showOnlineThumbnailNote: boolean;
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

  const tags = classifyTagsAssigned(s);
  const details: DetailItem[] = [];
  if (tags.tone === "amber" || tags.tone === "failed") {
    attention.push({ key: "tags", tone: tags.tone, text: tags.text, expandable: false });
  } else if (s.tags_assigned > 0) {
    // "muted" は原因つきの0件表示なので、件数0の項目として出さない。
    details.push({ key: "tags", text: tags.text });
  }

  if (s.ratings_from_star_tags > 0) {
    details.push({
      key: "star-ratings",
      text: `★タグから評価にした: ${s.ratings_from_star_tags}件`,
    });
  }
  if (s.thumbnails_skipped_online > 0) {
    details.push({
      key: "skipped-online",
      text: `オンライン動画の旧サムネイル: ${s.thumbnails_skipped_online}件 — 動画本体から自動生成します（移行対象外）。`,
    });
  }
  if (s.thumbnails_linked > 0) {
    details.push({ key: "linked", text: `旧サムネイル紐付け成功: ${s.thumbnails_linked}件` });
  }
  if (s.skipped > 0) {
    details.push({ key: "skipped", text: `スキップ (登録済み): ${s.skipped}件` });
  }
  if (s.clamped_scores > 0) {
    details.push({ key: "clamped", text: `score丸め: ${s.clamped_scores}件` });
  }

  return {
    registeredText: `登録: ${s.registered}件`,
    watchFoldersAddedText:
      s.watch_folders_added.length > 0
        ? `監視フォルダに追加: ${s.watch_folders_added.length}件`
        : null,
    attention,
    details,
    watchFolders: s.watch_folders_added,
    showOnlineThumbnailNote: s.thumbnails_skipped_online > 0,
  };
}
