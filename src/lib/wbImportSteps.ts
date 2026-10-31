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

/** 段階ごとの処理件数。total が null の間は分母が未確定。 */
export interface WbImportStageCount {
  processed: number;
  total: number | null;
}

export type WbImportStageCounts = Partial<Record<WbImportStageId, WbImportStageCount>>;

// 桁区切りは実行環境のロケールに依らないよう固定する。
function formatNumber(n: number): string {
  return n.toLocaleString("en-US");
}

/**
 * 段階の行の末尾に添える件数。出すものが無いときは空文字。
 * - 未着手、または件数がまだ届いていない: なし。
 * - 実行中: 分母ありは「x / y件」、分母なしは「x件」。
 * - 完了: 最終件数「N件」(分母があればそれ、無ければ処理済み件数)。
 */
export function formatStepCount(
  status: WbImportStepStatus,
  count: WbImportStageCount | undefined,
): string {
  if (status === "pending" || count === undefined) {
    return "";
  }
  if (status === "done") {
    return `${formatNumber(count.total ?? count.processed)}件`;
  }
  if (count.total === null) {
    return `${formatNumber(count.processed)}件`;
  }
  return `${formatNumber(count.processed)} / ${formatNumber(count.total)}件`;
}

/**
 * 進捗バーの値。実行中の段階に分母(1以上)があるときだけ返し、それ以外は null
 * (バーを出さない)。
 */
export function progressBarValue(
  stage: WbImportStageId | null,
  counts: WbImportStageCounts,
): { value: number; max: number } | null {
  if (stage === null) {
    return null;
  }
  const count = counts[stage];
  if (count === undefined || count.total === null || count.total <= 0) {
    return null;
  }
  return { value: Math.min(count.processed, count.total), max: count.total };
}
