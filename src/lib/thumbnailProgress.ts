import type { ThumbnailProgress } from "../types";

/**
 * Status bar text for thumbnail generation, or null when nothing is being
 * generated (`total === 0`) so the caller renders nothing.
 */
export function formatThumbnailProgress(p: ThumbnailProgress): string | null {
  if (p.total === 0) return null;
  return `サムネイル生成（${p.done}/${p.total}件）`;
}
