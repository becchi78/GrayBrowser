// LeftPane.tsx（JSXを含むコンポーネントファイル）にあった純粋関数
// effectiveMaxWidth を単体テスト可能にするため、JSX非依存のこのファイルに
// 切り出した。
export const MIN_WIDTH = 200;
export const MAX_WIDTH = 500;
// `.app`'s own `padding: var(--space-4)` (16px) on both
// left and right = 32px total, unrelated to this component but needed here
// to compute how much of the window's width is actually available to
// `.main-area` (App.css) before this sidebar even starts competing with
// `.video-list` for space.
const APP_HORIZONTAL_PADDING_PX = 32;
// `.video-list` の min-width(CSS の `--video-list-min-width`)を読めないとき
// (CSS の無いテスト環境)の既定値。実際の値はテーマごとに CSS が決め、
// 呼び出し側が読んで引数で渡す。ここの値は、行の左右padding 24px + gap 8px +
// サムネイル領域の最小幅 200px + 情報パネル 300px = 532px(枠の無い
// シネマの値)に合わせてある。
export const VIDEO_LIST_MIN_WIDTH_PX = 532;

/**
 * The widest this sidebar may be dragged to right now, given the window's
 * current width -- `MAX_WIDTH` (500px) unless the window is too narrow to
 * fit both that and `.video-list`'s own floor at the same time, in which
 * case this shrinks the effective ceiling instead (never below
 * `MIN_WIDTH`, so the sidebar itself never gets clamped to something
 * smaller than its own minimum).
 */
export function effectiveMaxWidth(
  windowInnerWidth: number,
  videoListMinWidth: number = VIDEO_LIST_MIN_WIDTH_PX,
): number {
  const available = windowInnerWidth - APP_HORIZONTAL_PADDING_PX - videoListMinWidth;
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, available));
}
