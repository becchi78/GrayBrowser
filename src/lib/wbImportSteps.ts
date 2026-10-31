// .wbインポート中の段階表示の状態判定。段階名はバックエンド
// (crates/gb-core の WbImportStage::as_str) が wb_import:stage で送る文字列と一致させる。

import type { WbImportStageId } from "../types";

export type WbImportStepStatus = "pending" | "active" | "done";

export type WbImportOutcome = "complete" | "failed";

export interface WbImportStep {
  key: WbImportStageId;
  label: string;
  status: WbImportStepStatus;
}

const STEP_DEFS: { key: WbImportStageId; label: string }[] = [
  { key: "reading", label: "読み込み" },
  { key: "registering", label: "動画の登録" },
  { key: "linking", label: "旧サムネイルの紐付け" },
];

export function isWbImportStageId(value: string): value is WbImportStageId {
  return STEP_DEFS.some((d) => d.key === value);
}

/**
 * 現在の段階と終了状態から各段階の状態を決める。実行中になるのは常に高々1つ。
 * - stage が null(最初の段階イベントが届く前)は全て未着手。
 * - 完了後は全て完了。失敗後は、失敗した段階とそれ以降を未着手のままにする
 *   (失敗した段階を「完了」と見せないため)。
 */
export function computeWbImportSteps(
  stage: WbImportStageId | null,
  outcome: WbImportOutcome | null,
): WbImportStep[] {
  const currentIndex = stage === null ? -1 : STEP_DEFS.findIndex((d) => d.key === stage);
  return STEP_DEFS.map((d, i) => {
    let status: WbImportStepStatus;
    if (outcome === "complete") {
      status = "done";
    } else if (i < currentIndex) {
      status = "done";
    } else if (i === currentIndex) {
      status = outcome === "failed" ? "pending" : "active";
    } else {
      status = "pending";
    }
    return { key: d.key, label: d.label, status };
  });
}

/** 進捗バーと「N / T件」を出すのは、件数のある「動画の登録」の段階だけ。 */
export function showsRowProgress(stage: WbImportStageId | null): boolean {
  return stage === "registering";
}
