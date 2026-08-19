import { useEffect, useState } from "react";
import { getThumbnails, getVideoProperties, playVideo } from "../api";
import { setWithEviction, THUMBNAIL_CACHE_MAX_ENTRIES } from "../lib/boundedCache";
import type { VideoDto, VideoPropertiesDto } from "../types";
import { StarRating } from "./StarRating";
import { TagEditor } from "./TagEditor";

interface Props {
  video: VideoDto;
  /** Shared across every row (owned by VideoList/ThumbnailGrid), keyed by video id. */
  cache: Map<string, string[]>;
  isSelected: boolean;
  onSelect: (video: VideoDto) => void;
}

// Every video shows exactly 6 evenly-spaced thumbnails (replaces the old
// grid's single thumbnail-per-cell).
const THUMBNAILS_PER_VIDEO = 6;

function formatDuration(seconds: number | null): string {
  if (seconds === null) return "不明";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

function formatFileSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
}

function formatBitrate(bps: number | null): string {
  if (bps === null) return "不明";
  return `${(bps / 1_000_000).toFixed(1)} Mbps`;
}

function formatResolution(width: number | null, height: number | null): string {
  return width !== null && height !== null ? `${width} × ${height}` : "不明";
}

function formatFps(fps: number | null): string {
  return fps === null ? "不明" : `${fps.toFixed(2)} fps`;
}

// Replaces both ThumbnailCell (single-thumbnail grid cell) and
// PropertiesPanel (separate detail sidebar). One VideoRow = one video =
// one list row: 6 thumbnails on the left, metadata/rating/tags on the
// right, no file-name heading and no explicit play button (ダブルクリック
// が再生の唯一の手段).
//
// Rendered with `key={video.id}` by its caller (ThumbnailGrid, same
// convention as the old PropertiesPanel) -- TanStack Virtual's row
// virtualization here maps one virtual row to exactly one video (no
// per-row grouping, unlike the old multi-column grid), so keying the
// component itself by video id lets React remount it fresh whenever the
// video shown at a given scroll position changes, resyncing `rating`'s
// lazy initializer without a manual effect.
export function VideoRow({ video, cache, isSelected, onSelect }: Props) {
  const [thumbnails, setThumbnails] = useState<string[] | null>(cache.get(video.id) ?? null);
  const [rating, setRating] = useState(video.rating);
  const [properties, setProperties] = useState<VideoPropertiesDto | null>(null);

  useEffect(() => {
    // The cache-hit case is already covered by useState's lazy initializer
    // above; this effect only needs to handle the actual fetch.
    if (!video.thumbnail_ready || thumbnails || cache.has(video.id)) {
      return;
    }
    let cancelled = false;
    getThumbnails(video.id)
      .then((urls) => {
        if (cancelled || !urls) return;
        // 上限（THUMBNAIL_CACHE_MAX_ENTRIES）を超えたら最も古いエントリを
        // 追い出す。大規模ライブラリを最後までスクロールしてもキャッシュが
        // 無制限に増え続けないようにするための対策。
        setWithEviction(cache, video.id, urls, THUMBNAIL_CACHE_MAX_ENTRIES);
        setThumbnails(urls);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [video.id, video.thumbnail_ready, thumbnails, cache]);

  useEffect(() => {
    let cancelled = false;
    getVideoProperties(video.id)
      .then((props) => {
        if (!cancelled) setProperties(props);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [video.id]);

  // `probed_at === null` covers both "not yet probed" and "probe failed
  // permanently" -- the backend doesn't distinguish these today (known
  // limitation). Showing "取得中..." is the common case; this is a
  // deliberate simplification, not a claim that it's always still in
  // progress.
  const stillProbing = properties === null || properties.probed_at === null;
  const isOffline = video.status === "offline";

  // サムネイルは元動画のアスペクト比を保ったまま表示する（バックエンドの
  // `extract_thumbnail`は`-vf
  // scale={width}:-1`で幅指定・高さ自動計算のため、生成されたWebP自体は既に
  // 正しい比率を保持している -- クロップしていたのはこのフロント側の固定
  // 200x150枠+`object-fit: cover`のみだった）。高さ150pxは固定のまま、幅を
  // 動画の実際の width/height 比から算出する。メタデータ未取得時
  // (`stillProbing`)は現行の200px（4:3相当）をフォールバックとして使う --
  // 6枚とも同一動画のサムネイルなので、算出した幅を全スロットに適用する。
  const THUMBNAIL_HEIGHT_PX = 150;
  const DEFAULT_THUMBNAIL_WIDTH_PX = 200;
  const thumbnailWidth =
    properties?.width && properties?.height
      ? Math.round((THUMBNAIL_HEIGHT_PX * properties.width) / properties.height)
      : DEFAULT_THUMBNAIL_WIDTH_PX;

  function handleDoubleClick() {
    if (isOffline) return;
    playVideo(video.file_path).catch(console.error);
  }

  return (
    <div
      className={
        "video-row" +
        (isSelected ? " video-row--selected" : "") +
        (isOffline ? " video-row--offline" : "")
      }
      data-testid="video-row"
    >
      {/* Single click selects (highlight only,
          no panel to open anymore); double click plays via the external
          player (disabled while offline). Both handlers live on the
          thumbnail area itself, not the row as a whole -- the metadata/
          rating/tag-editor side has its own interactive controls that must
          not also trigger row selection or playback. */}
      {/* `.video-row-thumbnails-wrap` is the
          non-scrolling `position: relative` containing block, shared by two
          absolutely-positioned overlays that must NOT move when the inner
          `.video-row-thumbnails` scrolls horizontally: the offline badge
          (moved here from `.video-row-thumbnails` itself, which used to be
          both the scroll container AND the badge's containing block -- the
          badge drifted out of view whenever a user scrolled the 6
          thumbnails sideways) and the new file-name label below. */}
      <div className="video-row-thumbnails-wrap" data-testid="video-row-thumbnails-wrap">
        <div
          className="video-row-thumbnails"
          data-testid="video-row-thumbnails"
          title={video.file_name}
          onClick={() => onSelect(video)}
          onDoubleClick={handleDoubleClick}
        >
          {Array.from({ length: THUMBNAILS_PER_VIDEO }, (_, i) => {
            const url = thumbnails?.[i];
            return (
              <div
                key={i}
                className="video-row-thumbnail-wrap"
                style={{ flexBasis: thumbnailWidth, width: thumbnailWidth }}
              >
                {url ? (
                  <img src={url} alt={video.file_name} loading="lazy" />
                ) : (
                  <div className="thumbnail-placeholder">生成中...</div>
                )}
              </div>
            );
          })}
        </div>
        {isOffline && (
          <span className="thumbnail-offline-badge thumbnail-offline-badge--overlay">
            ⚠ オフライン
          </span>
        )}
        {/* `pointer-events: none`（App.css側）でクリックを透過させ、
            すぐ下の`.video-row-thumbnails`のselect/play判定をそのまま維持する。
            title属性はここではなく`.video-row-thumbnails`側に持たせる:
            `pointer-events: none`の要素はブラウザのヒットテスト対象から外れ
            mouseoverを一切受け取れないため、この要素自身のtitleはツールチッ
            プとして表示されない（背後の`.video-row-thumbnails`がホバー対象
            になり、HTML仕様上その祖先のtitleが使われる）。 */}
        <span className="video-row-filename-overlay" data-testid="video-row-filename">
          {video.file_name}
        </span>
      </div>

      <div className="video-row-info">
        <dl className="video-row-properties" data-testid="video-row-properties">
          <dt>再生時間</dt>
          <dd>{formatDuration(video.duration)}</dd>
          <dt>ファイルサイズ</dt>
          <dd>{formatFileSize(video.file_size)}</dd>
          {stillProbing ? (
            <>
              <dt>詳細情報</dt>
              <dd>取得中...</dd>
            </>
          ) : (
            <>
              <dt>解像度</dt>
              <dd>{formatResolution(properties.width, properties.height)}</dd>
              <dt>映像コーデック</dt>
              <dd>{properties.video_codec ?? "不明"}</dd>
              <dt>音声コーデック</dt>
              <dd>{properties.audio_codec ?? "不明"}</dd>
              <dt>ビットレート</dt>
              <dd>{formatBitrate(properties.bitrate)}</dd>
              <dt>フレームレート</dt>
              <dd>{formatFps(properties.fps)}</dd>
            </>
          )}
        </dl>

        <StarRating videoId={video.id} value={rating} onChange={setRating} />
        <TagEditor videoId={video.id} />
      </div>
    </div>
  );
}
