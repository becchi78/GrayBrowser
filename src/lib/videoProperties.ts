import type { VideoPropertiesDto } from "../types";

// `probed_at === null` は「まだ取得していない」と「ffprobeに永続的に失敗した」
// の両方を含む（バックエンドは現状この2つを区別しない）。どちらも「取得が
// 終わっていない」として扱い、取り直しの対象にする。
export function needsPropertiesRefresh(properties: VideoPropertiesDto | null): boolean {
  return properties === null || properties.probed_at === null;
}

const THUMBNAIL_HEIGHT_PX = 150;
const DEFAULT_THUMBNAIL_WIDTH_PX = 200;

// サムネイルは元動画のアスペクト比を保って表示する。生成されたWebP自体が
// 既に正しい比率なので、高さ150pxを固定し、幅を動画の width/height 比から
// 算出する（固定枠+`object-fit: cover`だとクロップされる）。メタデータ未取得や
// 幅・高さが不明(null・0)のときは200px（4:3相当）をフォールバックにする。
export function thumbnailWidthPx(properties: VideoPropertiesDto | null): number {
  return properties?.width && properties?.height
    ? Math.round((THUMBNAIL_HEIGHT_PX * properties.width) / properties.height)
    : DEFAULT_THUMBNAIL_WIDTH_PX;
}
