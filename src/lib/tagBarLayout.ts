// computeVisibleChipCount, the TagBar overflow calculation, split out into
// this JSX非依存 file so it's unit-testable without a DOM/component harness --
// same pattern as src/lib/sidebarResize.ts (FolderSidebar's own resize-clamp
// math).
import { APP_HORIZONTAL_PADDING_PX } from "./sidebarResize.ts";

/**
 * ウィンドウ幅と各チップ(表示順どおり)の実測幅から、先頭から何個のチップが
 * バーに収まるかを返す。収まらなかった残りは呼び出し側が▾ドロップダウンに
 * 回す。ResizeObserverではなくwindowInnerWidthを直接受け取る純粋関数として
 * 切り出す(DOM計測自体はコンポーネント側で行う、sidebarResize.tsと同じ
 * 責務分離)。
 */
export function computeVisibleChipCount(
  windowInnerWidth: number,
  clearAllWidthPx: number,
  overflowButtonWidthPx: number,
  chipWidthsPx: number[],
): number {
  const available = windowInnerWidth - APP_HORIZONTAL_PADDING_PX - clearAllWidthPx;
  const total = chipWidthsPx.reduce((a, b) => a + b, 0);
  if (total <= available) {
    return chipWidthsPx.length; // ▾ボタン自体不要なケース
  }
  const budget = available - overflowButtonWidthPx;
  let used = 0;
  let count = 0;
  for (const w of chipWidthsPx) {
    if (used + w > budget) break;
    used += w;
    count++;
  }
  return count;
}
