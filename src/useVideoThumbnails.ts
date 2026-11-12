import { useEffect, useState } from "react";
import { getThumbnails } from "./api";
import { setWithEviction, THUMBNAIL_CACHE_MAX_ENTRIES } from "./lib/boundedCache";
import type { VideoDto } from "./types";

// 一覧の行に出すサムネイル(6枚分のURL)。行どうしで共有するキャッシュにあれば
// それを使い、無ければ thumbnail_ready の動画だけ取得してキャッシュに入れる。
// 取得できるまでは null。
export function useVideoThumbnails(
  video: VideoDto,
  thumbnailVersion: number,
  cache: Map<string, string[]>,
): string[] | null {
  // The version is in the key so images cached before a replacement are never
  // served again; the parent also remounts the row, which drops `thumbnails`.
  const cacheKey = `${video.id}:${thumbnailVersion}`;
  const [thumbnails, setThumbnails] = useState<string[] | null>(cache.get(cacheKey) ?? null);

  useEffect(() => {
    // The cache-hit case is already covered by useState's lazy initializer
    // above; this effect only needs to handle the actual fetch.
    if (!video.thumbnail_ready || thumbnails || cache.has(cacheKey)) {
      return;
    }
    let cancelled = false;
    getThumbnails(video.id)
      .then((urls) => {
        if (cancelled || !urls) return;
        // 上限（THUMBNAIL_CACHE_MAX_ENTRIES）を超えたら最も古いエントリを
        // 追い出す。大規模ライブラリを最後までスクロールしてもキャッシュが
        // 無制限に増え続けないようにするための対策。
        setWithEviction(cache, cacheKey, urls, THUMBNAIL_CACHE_MAX_ENTRIES);
        setThumbnails(urls);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [video.id, cacheKey, video.thumbnail_ready, thumbnails, cache]);

  return thumbnails;
}
