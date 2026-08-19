// `.wb`インポート結果ダイアログ（WbImportDialog.tsx）の「タグ付与」行が
// tags_assigned===0になる原因を区別するための、pure functionのロジック切り出し。
// src/lib/boundedCache.ts と同じパターン（OS/UIランタイムに依存しないロジックを
// src/lib/ に置き、node --experimental-strip-types --test で単体テストする）。
//
// tags_assigned===0 になる原因は少なくとも次の4通りがあり、区別しないと
// 「正常動作を失敗のように見せる」（元データにタグが無かっただけ／全行が
// 既登録スキップだっただけ）ことになりかねない一方、逆に「失敗を正常寄りに
// 見せる」（全行が取り込みエラーで失敗した）ことも避けなければならない。
//
// src-tauri/src/wb_import/pipeline.rs の import_all を見ると、1行の処理結果は
// 「Inserted（タグも書き込み済み）」か「Skipped（既登録、タグ含め一切
// 書き込まない）」の二択で、失敗した行（不正な日付形式・DB書き込みエラー等）
// は continue され、registered にも skipped にもカウントされない
// （db/queries.rs の import_wb_video はトランザクション内で行＋タグをまとめて
// 書き込み、失敗すれば丸ごとロールバックされる）。
import type { WbImportSummary } from "../types";

export type TagsAssignedTone = "success" | "muted" | "amber" | "failed";

export interface TagsAssignedDisplay {
  tone: TagsAssignedTone;
  text: string;
}

export function classifyTagsAssigned(s: WbImportSummary): TagsAssignedDisplay {
  if (s.tags_assigned > 0) {
    return { tone: "success", text: `タグ付与: ${s.tags_assigned}件` };
  }
  if (s.source_tag_count === 0) {
    return { tone: "muted", text: "タグ付与: 0件（元データにタグがありませんでした）" };
  }
  if (s.registered === 0 && s.skipped > 0) {
    return {
      tone: "muted",
      text: "タグ付与: 0件（対象の動画はすべて既に登録済みのため、新規のタグ付与はありませんでした）",
    };
  }
  if (s.registered === 0 && s.skipped === 0) {
    // WbImportSummaryに行レベルの失敗件数を表す専用フィールドが無いため、
    // 「新規挿入も既登録スキップも0件なのに元データにタグは存在した」ことから
    // 全行が取り込みエラーで失敗したと推定する(直接のフィールドによる判定ではない)。
    return {
      tone: "failed",
      text: `タグ付与: 0件 — インポートに失敗した可能性があります（元データには${s.source_tag_count}件のタグがありましたが、動画が1件も取り込まれていません）`,
    };
  }
  return {
    tone: "amber",
    text: `タグ付与: 0件 — 元データには${s.source_tag_count}件のタグがありましたが、付与されませんでした（要確認）`,
  };
}
