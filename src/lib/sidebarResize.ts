// FolderSidebar.tsx（JSXを含むコンポーネントファイル）にあった純粋関数
// effectiveMaxWidth を単体テスト可能にするため、JSX非依存のこのファイルに
// 切り出した。src/lib/paths.ts・src/lib/boundedCache.ts と同じパターン
// （コンポーネント側からimportして使う）。
export const MIN_WIDTH = 200;
export const MAX_WIDTH = 500;
// `.app`'s own `padding: 1em` (16px at :root's `font-size: 16px`) on both
// left and right = 32px total, unrelated to this component but needed here
// to compute how much of the window's width is actually available to
// `.main-area` (App.css) before this sidebar even starts competing with
// `.video-list` for space.
export const APP_HORIZONTAL_PADDING_PX = 32;
// App.css's `.video-list`'s own `min-width` -- kept as a literal constant
// here (not imported; this codebase has no CSS-values-in-JS bridge) so this
// component doesn't drag this sidebar wide enough, at a narrow window, to
// force `.video-list` below its own floor. Must be kept in sync with that
// rule's `min-width` -- see its own comment for where 532 comes from.
export const VIDEO_LIST_MIN_WIDTH_PX = 532;

/**
 * The widest this sidebar may be dragged to right now, given the window's
 * current width -- `MAX_WIDTH` (500px) unless the window is too narrow to
 * fit both that and `.video-list`'s own floor at the same time, in which
 * case this shrinks the effective ceiling instead (never below
 * `MIN_WIDTH`, so the sidebar itself never gets clamped to something
 * smaller than its own minimum).
 */
export function effectiveMaxWidth(windowInnerWidth: number): number {
  const available = windowInnerWidth - APP_HORIZONTAL_PADDING_PX - VIDEO_LIST_MIN_WIDTH_PX;
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, available));
}
