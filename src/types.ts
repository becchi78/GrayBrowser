// Mirrors of the Rust DTOs returned by src-tauri/src/commands/*.rs. Field
// names are kept as-is (snake_case) since Tauri's JSON payloads are not
// case-converted; only top-level command *argument names* are camelCased.

export interface VideoDto {
  id: string;
  file_path: string;
  file_name: string;
  file_size: number;
  duration: number | null;
  quick_hash: string;
  full_hash: string | null;
  status: "online" | "offline";
  rating: number;
  created_at: string;
  thumbnail_ready: boolean;
  /** 自動の再試行を使い切っても、まだサムネイルがない状態。 */
  thumbnail_failed: boolean;
}

export interface SkippedFileDto {
  id: number;
  file_path: string;
  file_name: string;
  reason: string;
  detected_char: string | null;
  detected_at: string;
  /** 元のファイルが確実にないと確認できた行。 */
  file_missing: boolean;
}

export interface MissingRegistrationResult {
  /** `false` のとき、.wbの情報がなくタグ・評価は付いていない。 */
  carried_wb_data: boolean;
}

export interface FfmpegStatusDto {
  available: boolean;
  ffmpeg_version: string | null;
  ffprobe_version: string | null;
}

export interface TagDto {
  id: number;
  name: string;
}

export interface TagWithCountDto extends TagDto {
  video_count: number;
}

export interface AssignTagResultDto {
  tag: TagDto;
  newly_tagged: number;
  already_tagged: number;
  missing: number;
}

// Mirrors src-tauri's SortFieldParam/SortDirectionParam (serde
// rename_all = "snake_case") -- these are the only valid strings the
// list_videos command accepts for sorting.
export type SortField =
  | "file_name"
  | "created_at"
  | "updated_date"
  | "rating"
  | "file_size"
  | "folder";
export type SortDirection = "asc" | "desc";

export interface VideoPropertiesDto {
  width: number | null;
  height: number | null;
  video_codec: string | null;
  audio_codec: string | null;
  bitrate: number | null;
  fps: number | null;
  /** null means "not yet probed" -- render as a pending state, not blank fields. */
  probed_at: string | null;
}

// Mirrors src-tauri/src/events.rs's WbImportSummary, the payload of the
// wb_import:complete event emitted once by wb_import::pipeline::run_wb_import.
export interface WbImportSummary {
  /** Number of movie rows in the `.wb`. Equals
   * `registered + skipped + skipped_invalid_name + failed_rows`. */
  total: number;
  /** Always `registered_online + registered_offline`. */
  registered: number;
  registered_online: number;
  registered_offline: number;
  /** Rows that could not be imported (bad date, failed database write). */
  failed_rows: number;
  skipped: number;
  /** Rows not registered because the file name has a machine-dependent
   * character; they are listed in the unregistered-files panel. */
  skipped_invalid_name: number;
  clamped_scores: number;
  tags_assigned: number;
  /** Total tags present in the raw `.wb` source across every row, so the
   * import dialog can tell "tags_assigned === 0 because the source had
   * none" apart from "...because something went wrong". */
  source_tag_count: number;
  /** Newly registered videos whose rating came from a ★-only tag. */
  ratings_from_star_tags: number;
  thumbnails_linked: number;
  thumbnails_failed: number;
  thumbnails_unmatched: number;
  thumbnails_skipped_online: number;
  /** Folders this import newly added to the watch-folder list. */
  watch_folders_added: string[];
}

// Payload of the wb_import:stage event: gb_core::wb_import::WbImportStage::as_str.
export type WbImportStageId = "reading" | "registering" | "linking";

// Mirrors src-tauri/src/events.rs's TauriWbImportNotifier::notify_progress
// anonymous Progress struct, the payload of the wb_import:progress event.
// `total` is null while the stage cannot know its size yet.
export interface WbImportProgress {
  stage: WbImportStageId;
  processed: number;
  total: number | null;
}

// Mirrors src-tauri/src/dedup/mod.rs's DuplicateGroupMember/DuplicateGroupKind/
// DuplicateGroup. DuplicateGroupKind is #[serde(rename_all = "snake_case")]
// over its PascalCase Rust variant names.
export interface DuplicateGroupMember {
  video_id: string;
  file_path: string;
  file_name: string;
  file_size: number;
  status: "online" | "offline";
  created_at: string;
  /** 秒。未取得なら null。 */
  duration: number | null;
  /** probe 未実施・取得不能なら null。 */
  width: number | null;
  height: number | null;
}

export type DuplicateGroupKind =
  | "quick_hash_confirmed"
  | "path_collision_confirmed"
  | "path_collision_unconfirmed"
  | "quick_hash_candidate_unverified";

export interface DuplicateGroup {
  kind: DuplicateGroupKind;
  members: DuplicateGroupMember[];
}

// Mirrors src-tauri/src/commands/generation_retry_cmds.rs's ExhaustedThumbnailDto/
// ExhaustedMetadataDto/GenerationFailuresDto: videos whose automatic
// thumbnail/metadata generation has exhausted its retry budget.
export interface ExhaustedThumbnailDto {
  id: string;
  file_path: string;
  file_name: string;
  thumbnail_attempts: number;
}

export interface ExhaustedMetadataDto {
  id: string;
  file_path: string;
  file_name: string;
  metadata_attempts: number;
}

export interface GenerationFailuresDto {
  thumbnail_failures: ExhaustedThumbnailDto[];
  metadata_failures: ExhaustedMetadataDto[];
}

// Mirrors src-tauri/src/commands/settings_cmds.rs's RenameWatchFolderResult,
// the return payload of `rename_watch_folder`: the updated folder list plus
// how many `videos` rows were rewritten vs. left untouched due to a path
// collision, so FolderDialog can surface both without a second round trip.
export interface RenameWatchFolderResult {
  folders: string[];
  renamed_count: number;
  collision_skipped_count: number;
}

export interface ScanSummary {
  scanned: number;
  registered: number;
  /** A known path reconciled: content changed and/or reconnected offline -> online. */
  reconciled: number;
  /** A known path whose mtime+file_size still matched -- no DB write occurred. */
  unchanged: number;
  skipped: number;
  /** A known online video confirmed missing this scan and flipped to offline. */
  went_offline: number;
  /** An offline row matched by quick_hash+file_size at a new path and reactivated there. */
  reactivated: number;
  /** A path-follow candidate existed but its target path collided with an online row; registered as a new row instead. */
  collisions: number;
  /** Watch folders skipped because they could not be reached; they are neither scanned nor marked offline. */
  unreachable_folders: string[];
}

/** `total === 0` means no thumbnail generation is in progress. */
export interface ThumbnailProgress {
  done: number;
  total: number;
}

export interface RenameSkippedFileResult {
  new_file_name: string;
  registered: boolean;
}

export interface LogEntryDto {
  /** ERROR / WARN / INFO / DEBUG / TRACE / UNKNOWN */
  level: string;
  target: string;
  message: string;
}

export interface LogChunkDto {
  entries: LogEntryDto[];
  next_offset: number;
  /** true のとき、保持している分を捨ててから entries を使う。 */
  reset: boolean;
}
