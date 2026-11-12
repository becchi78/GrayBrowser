import {
  formatBitrate,
  formatDuration,
  formatFileSize,
  formatFps,
  formatResolution,
} from "../lib/format";
import type { VideoDto, VideoPropertiesDto } from "../types";

// 一覧の行の右側に出すプロパティ表。再生時間とファイルサイズはカタログの値で
// 常に出し、それ以外はプロパティの取得が終わるまで「取得中...」にまとめる。
export function VideoRowProperties({
  video,
  properties,
  stillProbing,
}: {
  video: VideoDto;
  properties: VideoPropertiesDto | null;
  stillProbing: boolean;
}) {
  return (
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
  );
}
