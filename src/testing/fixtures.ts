// Node 組み込みの test runner(src/lib のテスト)からも読み込まれるため、
// vitest・React・DOM に依存するものはここに置かない(それらは fakes.ts など別ファイルへ)。
import type { DuplicateGroupMember, UnmatchedThumbnailDto, WbImportSummary } from "../types.ts";

/**
 * 全項目が 0・空の .wb インポート集計。テストで意味を持たせる項目だけを overrides で上書きする。
 */
export function makeSummary(overrides: Partial<WbImportSummary> = {}): WbImportSummary {
  return {
    total: 0,
    registered: 0,
    registered_online: 0,
    registered_offline: 0,
    failed_rows: 0,
    skipped: 0,
    skipped_invalid_name: 0,
    clamped_scores: 0,
    tags_assigned: 0,
    source_tag_count: 0,
    thumbnails_linked: 0,
    thumbnails_linked_by_name: 0,
    thumbnails_failed: 0,
    thumbnails_unmatched: 0,
    thumbnails_unmatched_empty_hash: 0,
    watch_folders_added: [],
    watch_folders_skipped_roots: [],
    volume_label_resolved: 0,
    volume_label_unresolved: 0,
    volume_label_unresolved_labels: [],
    ...overrides,
  };
}

/** `makeMember(id)` が返すメンバーの file_path。 */
export const memberPath = (id: string) => `C:\\videos\\${id}.mp4`;

/** 重複グループのメンバー。id から file_path・file_name が決まる。 */
export function makeMember(
  id = "id",
  overrides: Partial<DuplicateGroupMember> = {},
): DuplicateGroupMember {
  return {
    video_id: id,
    file_path: memberPath(id),
    file_name: `${id}.mp4`,
    file_size: 100 * 1024 * 1024,
    status: "online",
    created_at: "2024-01-01 00:00:00",
    duration: 60,
    width: 1920,
    height: 1080,
    ...overrides,
  };
}

/** 対応する動画が見つからない未対応サムネイルの1行。 */
export const UNMATCHED_ROW: UnmatchedThumbnailDto = {
  folder_path: "C:\\old",
  file_name: "a.jpg",
  kind: "no_matching_video",
  default_search_term: "a",
};
