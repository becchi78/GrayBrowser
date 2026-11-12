import { useState, type MouseEvent } from "react";
import type { ClickModifiers } from "../lib/multiSelect";
import { thumbnailPlaceholderState, THUMBNAIL_PLACEHOLDER_LABELS } from "../lib/thumbnailPlaceholder";
import { thumbnailWidthPx } from "../lib/videoProperties";
import { useIsPathUnreachable } from "../reachabilityContext";
import type { VideoDto } from "../types";
import { useStartPlay } from "../useStartPlay";
import { useVideoProperties } from "../useVideoProperties";
import { useVideoThumbnails } from "../useVideoThumbnails";
import { StarRating } from "./StarRating";
import { TagEditor } from "./TagEditor";
import { VideoRowProperties } from "./VideoRowProperties";

interface Props {
  video: VideoDto;
  /** Bumped when this video's thumbnail files were replaced; part of the cache key. */
  thumbnailVersion: number;
  /** Shared across every row (owned by VideoList/ThumbnailGrid), keyed by video id. */
  cache: Map<string, string[]>;
  /** Bumped by the parent on every list re-fetch; see useVideoProperties. */
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
  const thumbnails = useVideoThumbnails(video, thumbnailVersion, cache);
  const [rating, setRating] = useState(video.rating);
  const { properties, stillProbing } = useVideoProperties(video.id, catalogVersion);

  // 保存先のフォルダに今届かない場合も、DBの状態は変えずに通常のオフラインと
  // 同じ見た目・同じ操作制限にする（フォルダが戻れば自動で元に戻る）。
  const folderUnreachable = useIsPathUnreachable(video.file_path);
  const isOffline = video.status === "offline" || folderUnreachable;
  const placeholderState = thumbnailPlaceholderState(video, isOffline);

  // 6枚とも同一動画のサムネイルなので、算出した幅を全スロットに適用する。
  const thumbnailWidth = thumbnailWidthPx(properties);

  // ダブルクリックでの再生。失敗したときだけ理由を行内に表示し、開始中の状態は
  // 使わない（理由は再生失敗の表示に付けたコメント）。
  const { playError, handlePlay } = useStartPlay(video.file_path, isOffline);

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
          onDoubleClick={handlePlay}
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
        <VideoRowProperties video={video} properties={properties} stillProbing={stillProbing} />

        <StarRating videoId={video.id} value={rating} onChange={setRating} />
        <TagEditor videoId={video.id} tagsVersion={tagsVersion} />
      </div>
    </div>
  );
}
