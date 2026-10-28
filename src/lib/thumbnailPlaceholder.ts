export type ThumbnailPlaceholderState = "failed" | "offline" | "generating";

export const THUMBNAIL_PLACEHOLDER_LABELS: Record<
  ThumbnailPlaceholderState,
  string
> = {
  failed: "生成失敗",
  offline: "オフライン",
  generating: "生成中...",
};

/**
 * State of the placeholder shown where a thumbnail is missing. Offline videos
 * are never queued for generation, so they would otherwise stay "generating"
 * forever. A recorded failure wins over offline because it is the more
 * specific fact about this video.
 */
export function thumbnailPlaceholderState(
  video: { thumbnail_failed: boolean },
  isOffline: boolean,
): ThumbnailPlaceholderState {
  if (video.thumbnail_failed) return "failed";
  if (isOffline) return "offline";
  return "generating";
}
