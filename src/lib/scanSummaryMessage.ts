import type { ScanSummary } from "../types";

// 到達できないフォルダの件数は、0 件のときは出さない(通常の結果行を
// 騒がせないため)。飛ばしたフォルダがあると、動画が増えない理由が
// 結果行だけでは分からなくなるので、あるときだけ末尾に添える。
export function buildScanSummaryMessage(summary: ScanSummary): string {
  const base =
    `検出: ${summary.scanned} / 登録: ${summary.registered} / 更新: ${summary.reconciled} / ` +
    `変化なし: ${summary.unchanged} / スキップ: ${summary.skipped} / ` +
    `オフライン化: ${summary.went_offline} / 復帰: ${summary.reactivated}`;
  const unreachable = summary.unreachable_folders.length;
  return unreachable > 0 ? `${base} / 到達できないため飛ばしたフォルダ: ${unreachable} 件` : base;
}
