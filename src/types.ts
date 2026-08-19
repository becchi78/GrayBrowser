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
}

export interface SkippedFileDto {
  id: number;
  file_path: string;
  file_name: string;
  reason: string;
  detected_char: string | null;
  detected_at: string;
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

// Mirrors src-tauri's SortFieldParam/SortDirectionParam (serde
// rename_all = "snake_case") -- these are the only valid strings the
// list_videos command accepts for sorting.
export type SortField = "file_name" | "created_at" | "updated_date" | "rating";
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
  registered: number;
  skipped: number;
  clamped_scores: number;
  tags_assigned: number;
  /** Total tags present in the raw `.wb` source across every row, so the
   * import dialog can tell "tags_assigned === 0 because the source had
   * none" apart from "...because something went wrong". */
  source_tag_count: number;
  thumbnails_linked: number;
  thumbnails_failed: number;
  thumbnails_unmatched: number;
}

// Mirrors src-tauri/src/events.rs's TauriWbImportNotifier::notify_progress
// anonymous Progress struct, the payload of the wb_import:progress event.
export interface WbImportProgress {
  processed: number;
  total: number;
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
}

export type DuplicateGroupKind =
  | "quick_hash_confirmed"
  | "path_collision_confirmed"
  | "path_collision_unconfirmed";

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
}
