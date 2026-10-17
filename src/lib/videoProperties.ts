import type { VideoPropertiesDto } from "../types";

// `probed_at === null` は「まだ取得していない」と「ffprobeに永続的に失敗した」
// の両方を含む（バックエンドは現状この2つを区別しない）。どちらも「取得が
// 終わっていない」として扱い、取り直しの対象にする。
export function needsPropertiesRefresh(properties: VideoPropertiesDto | null): boolean {
  return properties === null || properties.probed_at === null;
}
