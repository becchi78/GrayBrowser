import type { ThumbnailProgress } from "../types";

/**
 * True for a value that ends a generation run (everything done) or says there
 * is nothing to generate; such a value must never be dropped by throttling.
 */
export function isFinalProgress(p: ThumbnailProgress): boolean {
  return p.total === 0 || p.done >= p.total;
}

/**
 * Status bar text for thumbnail generation, or null when nothing is being
 * generated (`total === 0`) so the caller renders nothing.
 */
export function formatThumbnailProgress(p: ThumbnailProgress): string | null {
  if (p.total === 0) return null;
  return `サムネイル生成（${p.done}/${p.total}件）`;
}
