// `.wb`インポート結果ダイアログ（WbImportDialog.tsx）の「タグをインポート」行が
// tags_assigned===0になる原因を区別するための、pure functionのロジック切り出し。
// src/lib/boundedCache.ts と同じパターン（OS/UIランタイムに依存しないロジックを
// src/lib/ に置き、node --experimental-strip-types --test で単体テストする）。
//
// tags_assigned===0 になる原因は少なくとも次の4通りがあり、区別しないと
// 「正常動作を失敗のように見せる」（元データにタグが無かっただけ／全行が
// 既登録スキップだっただけ）ことになりかねない一方、逆に「失敗を正常寄りに
// 見せる」（全行が取り込みエラーで失敗した）ことも避けなければならない。
//
// src-tauri/src/wb_import/pipeline.rs の import_all では、1行の処理結果は
// 「Inserted（タグも書き込み済み）」「Skipped（既登録、タグ含め一切書き込まない）」
// 「機種依存文字で未登録」「失敗（不正な日付形式・DB書き込みエラー等。
// failed_rows に数える）」のどれか1つ。DB書き込みはトランザクション内で
// 行＋タグをまとめて行い、失敗すれば丸ごとロールバックされる。
import type { WbImportSummary } from "../types";

export type TagsAssignedTone = "success" | "muted" | "amber" | "failed";

export interface TagsAssignedDisplay {
  tone: TagsAssignedTone;
  text: string;
}

export function classifyTagsAssigned(s: WbImportSummary): TagsAssignedDisplay {
  if (s.tags_assigned > 0) {
    return { tone: "success", text: `タグをインポート: ${s.tags_assigned}件` };
  }
  if (s.source_tag_count === 0) {
    return { tone: "muted", text: "タグをインポート: 0件（元データにタグがありませんでした）" };
  }
  if (s.registered === 0 && s.skipped > 0) {
    return {
      tone: "muted",
      text: "タグをインポート: 0件（対象の動画はすべて既に登録済みのため、新規のタグ付与はありませんでした）",
    };
  }
  if (s.registered === 0 && s.skipped === 0 && s.skipped_invalid_name > 0) {
    // 全行が機種依存文字のファイル名で登録されなかった場合。取り込みの失敗ではなく
    // 仕様どおりの結果なので、「失敗した可能性」とは表示しない。
    return {
      tone: "muted",
      text: "タグをインポート: 0件（対象の動画はすべて機種依存文字のファイル名のため登録されませんでした）",
    };
  }
  if (s.registered === 0 && s.skipped === 0 && s.failed_rows > 0) {
    // 新規登録も既登録スキップも0件で、取り込めなかった行がある。
    return {
      tone: "failed",
      text: `タグをインポート: 0件 — インポートに失敗した可能性があります（元データには${s.source_tag_count}件のタグがありましたが、動画が1件も取り込まれていません）`,
    };
  }
  return {
    tone: "amber",
    text: `タグをインポート: 0件 — 元データには${s.source_tag_count}件のタグがありましたが、付与されませんでした（要確認）`,
  };
}
