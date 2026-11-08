import { useEffect, useState, type MouseEvent } from "react";
import { getThumbnails, getVideoProperties, playVideo } from "../api";
import { formatDuration, formatFileSize, formatResolution } from "../lib/format";
import { setWithEviction, THUMBNAIL_CACHE_MAX_ENTRIES } from "../lib/boundedCache";
import type { ClickModifiers } from "../lib/multiSelect";
import { thumbnailPlaceholderState, THUMBNAIL_PLACEHOLDER_LABELS } from "../lib/thumbnailPlaceholder";
import { needsPropertiesRefresh } from "../lib/videoProperties";
import { useIsPathUnreachable } from "../reachabilityContext";
import type { VideoDto, VideoPropertiesDto } from "../types";
import { StarRating } from "./StarRating";
import { TagEditor } from "./TagEditor";

interface Props {
  video: VideoDto;
  /** Bumped when this video's thumbnail files were replaced; part of the cache key. */
  thumbnailVersion: number;
  /** Shared across every row (owned by VideoList/ThumbnailGrid), keyed by video id. */
  cache: Map<string, string[]>;
  /** Bumped by the parent on every list re-fetch; see the properties effect. */
  catalogVersion: number;
  /** Bumped by the parent when tags change outside this row's own TagEditor. */
  tagsVersion: number;
  isSelected: boolean;
  /** True only while 2+ rows are selected; styled apart from a lone selection. */
  isMultiSelected: boolean;
  onSelect: (video: VideoDto, mods: ClickModifiers) => void;
  onContextMenu: (video: VideoDto, e: MouseEvent) => void;
}

// Every video shows exactly 6 evenly-spaced thumbnails.
const THUMBNAILS_PER_VIDEO = 6;

function formatBitrate(bps: number | null): string {
  if (bps === null) return "不明";
  return `${(bps / 1_000_000).toFixed(1)} Mbps`;
}

function formatFps(fps: number | null): string {
  return fps === null ? "不明" : `${fps.toFixed(2)} fps`;
}

// One VideoRow = one video = one list row: 6 thumbnails on the left,
// metadata/rating/tags on the right, no explicit play button (ダブルクリック
// が再生の唯一の手段).
//
// The caller renders it with `key={video.id}`. The virtualizer maps one
// virtual row to exactly one video, so keying by video id makes React remount
// the row whenever the video at a given scroll position changes, resyncing
// `rating`'s lazy initializer without a manual effect.
export function VideoRow({
  video,
  thumbnailVersion,
  cache,
  catalogVersion,
  tagsVersion,
  isSelected,
  isMultiSelected,
  onSelect,
  onContextMenu,
}: Props) {
  // The version is in the key so images cached before a replacement are never
  // served again; the parent also remounts the row, which drops `thumbnails`.
  const cacheKey = `${video.id}:${thumbnailVersion}`;
  const [thumbnails, setThumbnails] = useState<string[] | null>(cache.get(cacheKey) ?? null);
  const [rating, setRating] = useState(video.rating);
  const [properties, setProperties] = useState<VideoPropertiesDto | null>(null);
  const [isStartingPlay, setIsStartingPlay] = useState(false);
  const [playError, setPlayError] = useState<string | null>(null);

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

  // `probed_at === null` covers both "not yet probed" and "probe failed
  // permanently" -- the backend doesn't distinguish these today (known
  // limitation). Showing "取得中..." is the common case; this is a
  // deliberate simplification, not a claim that it's always still in
  // progress.
  const stillProbing = needsPropertiesRefresh(properties);

  // バックグラウンドのメタデータ取得は1件終わるごとに一覧の更新通知を出すが、
  // 行は再マウントされないため、マウント時の一度きりの取得では、その時点で
  // 未取得だった行が「取得中...」のまま固まる。そこで一覧の更新（catalogVersion）
  // を契機に、未取得の行だけ取り直す。stillProbing（boolean）を依存に使うのは、
  // 未取得のまま応答が返っても値が変わらず、effectがループしないようにするため。
  // ffprobeに永続的に失敗した行は更新のたびに取り直すが、対象は仮想スクロールで
  // 描画中の数行に限られ負荷は小さい。ポーリングはせずイベント駆動にそろえる。
  useEffect(() => {
    if (!stillProbing) return;
    let cancelled = false;
    getVideoProperties(video.id)
      .then((props) => {
        if (!cancelled) setProperties(props);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [video.id, catalogVersion, stillProbing]);
  // 保存先のフォルダに今届かない場合も、DBの状態は変えずに通常のオフラインと
  // 同じ見た目・同じ操作制限にする（フォルダが戻れば自動で元に戻る）。
  const folderUnreachable = useIsPathUnreachable(video.file_path);
  const isOffline = video.status === "offline" || folderUnreachable;
  const placeholderState = thumbnailPlaceholderState(video, isOffline);

  // サムネイルは元動画のアスペクト比を保って表示する。生成されたWebP自体が
  // 既に正しい比率なので、高さ150pxを固定し、幅を動画の width/height 比から
  // 算出する（固定枠+`object-fit: cover`だとクロップされる）。メタデータ未取得時
  // (`stillProbing`)は200px（4:3相当）をフォールバックにする。6枚とも同一動画の
  // サムネイルなので、算出した幅を全スロットに適用する。
  const THUMBNAIL_HEIGHT_PX = 150;
  const DEFAULT_THUMBNAIL_WIDTH_PX = 200;
  const thumbnailWidth =
    properties?.width && properties?.height
      ? Math.round((THUMBNAIL_HEIGHT_PX * properties.width) / properties.height)
      : DEFAULT_THUMBNAIL_WIDTH_PX;

  // 再生の開始は外部プレーヤーの起動待ち（ネットワーク先では数十秒）になりうる。
  // 開始中の二重ダブルクリックでバックグラウンドの起動処理を積み上げないよう
  // 無視し、失敗したときだけ理由を行内に表示する。
  async function handleDoubleClick() {
    if (isOffline || isStartingPlay) return;
    setPlayError(null);
    setIsStartingPlay(true);
    try {
      await playVideo(video.file_path);
    } catch (e) {
      console.error(e);
      setPlayError(`再生に失敗しました: ${String(e)}`);
    } finally {
      setIsStartingPlay(false);
    }
  }

  // metaKey is treated as Ctrl so the same gesture works on macOS keyboards.
  function handleSelectClick(e: MouseEvent) {
    onSelect(video, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
  }

  return (
    <div
      className={
        "video-row" +
        (isSelected ? " video-row--selected" : "") +
        (isMultiSelected ? " video-row--multi-selected" : "") +
        (isOffline ? " video-row--offline" : "")
      }
      data-testid="video-row"
      onContextMenu={(e) => onContextMenu(video, e)}
    >
      {/* Single click selects (highlight only); double click plays via the
          external player (disabled while offline). Both handlers live on the
          thumbnail area itself, not the row as a whole -- the metadata/
          rating/tag-editor side has its own interactive controls that must
          not also trigger row selection or playback. */}
      {/* `.video-row-thumbnails-wrap` is the non-scrolling
          `position: relative` containing block for the overlays (offline
          badge, file-name label) that must NOT move when the inner
          `.video-row-thumbnails` scrolls horizontally. */}
      <div className="video-row-thumbnails-wrap" data-testid="video-row-thumbnails-wrap">
        <div
          className="video-row-thumbnails"
          data-testid="video-row-thumbnails"
          title={video.file_name}
          onClick={handleSelectClick}
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
                  <div
                    className={`thumbnail-placeholder thumbnail-placeholder--${placeholderState}`}
                  >
                    {THUMBNAIL_PLACEHOLDER_LABELS[placeholderState]}
                  </div>
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
        {/* ファイル名はマウスで選択してコピーできるよう、クリックを受け取る
            （App.css側で文字幅だけに縮め、サムネイル上端を覆わない）。
            このラベルは`.video-row-thumbnails`の兄弟要素でイベントが伝播しない
            ため、行選択はここで明示的に呼ぶ。ダブルクリックは再生に結びつけず、
            ブラウザ標準の単語選択を優先する。ラベルがホバーを受け取るので、
            title属性もこの要素自身に持たせる。 */}
        <span
          className="video-row-filename-overlay"
          data-testid="video-row-filename"
          title={video.file_name}
          onClick={handleSelectClick}
        >
          {video.file_name}
        </span>
        {/* 再生失敗の表示は固定高の`.video-row-info`の外に置く。中に置くと
            行が増えた分だけ縦スクロールバーが出て内容幅が縮み、プロパティ表の
            ラベルが折り返して高さが跳ねるため。再生開始中の表示は、一瞬で消えて
            読めないので出さない。 */}
        {playError && (
          <span className="video-row-play-status video-row-play-status--error" role="alert">
            {playError}
          </span>
        )}
      </div>

      <div className="video-row-info">
        <dl className="video-row-properties" data-testid="video-row-properties">
          <dt>再生時間</dt>
          <dd>{formatDuration(video.duration)}</dd>
          <dt>ファイルサイズ</dt>
          <dd>{formatFileSize(video.file_size)}</dd>
          {stillProbing || properties === null ? (
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
        <TagEditor videoId={video.id} tagsVersion={tagsVersion} />
      </div>
    </div>
  );
}
