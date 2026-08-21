// Thin wrappers around invoke() -- every Tauri command call in the app goes
// through here, so command names and the Rust snake_case -> JS camelCase
// argument-name conversion are only dealt with in one place.

import { invoke } from "@tauri-apps/api/core";
import type {
  DuplicateGroup,
  FfmpegStatusDto,
  GenerationFailuresDto,
  RenameWatchFolderResult,
  ScanSummary,
  SkippedFileDto,
  SortDirection,
  SortField,
  TagDto,
  VideoDto,
  VideoPropertiesDto,
} from "./types";

export interface ListVideosOptions {
  /** Raw, unparsed search-box string -- term splitting happens in gb_core::search. */
  search?: string;
  sortField?: SortField;
  sortDirection?: SortDirection;
  /** AND filter: a video must carry every listed tag id. */
  tagIds?: number[];
  /** Restricts to videos under this watch folder; `null`/omitted = every folder ("すべて"). */
  folderPath?: string | null;
}

// Keys are always present (explicit null rather than omitted) so Tauri's
// per-argument deserialization never has to guess at a missing key's intent.
// This matters in particular for `folderPath`: the "すべて" (no filter)
// selection in FolderSidebar must send an explicit `null`, not omit the key.
// This follows the same explicit-null pattern as every other optional
// argument above (design intent, not something newly verified here) -- the
// real Tauri IPC boundary behavior here is still unverified: a live
// WebDriver session against a real built app could not get far enough to
// click through FolderSidebar, because `list_watch_folders`'s invoke() call
// never resolved within the session. `folder_path`/`folderPath` wiring
// itself was only confirmed by code review, `cargo test`'s backend
// coverage (`src-tauri/tests/db_folder_filter.rs`), and a DOM structure
// check (FolderSidebar/main-area render) -- not by observing this specific
// null-vs-omitted behavior cross the real IPC boundary end to end.
export const listVideos = (opts?: ListVideosOptions) =>
  invoke<VideoDto[]>("list_videos", {
    search: opts?.search ?? null,
    sortField: opts?.sortField ?? null,
    sortDirection: opts?.sortDirection ?? null,
    tagIds: opts?.tagIds ?? null,
    folderPath: opts?.folderPath ?? null,
  });
export const assignTag = (videoId: string, tagName: string) =>
  invoke<TagDto>("assign_tag", { videoId, tagName });
export const removeTag = (videoId: string, tagId: number) =>
  invoke<void>("remove_tag", { videoId, tagId });
export const listTagsForVideo = (videoId: string) =>
  invoke<TagDto[]>("list_tags_for_video", { videoId });
export const listAllTags = () => invoke<TagDto[]>("list_all_tags");
export const getTagBarPinnedTagIds = () => invoke<number[]>("get_tag_bar_pinned_tag_ids");
export const setTagBarPinnedTagIds = (tagIds: number[]) =>
  invoke<void>("set_tag_bar_pinned_tag_ids", { tagIds });
export const setRating = (videoId: string, rating: number) =>
  invoke<void>("set_rating", { videoId, rating });
export const getVideoProperties = (videoId: string) =>
  invoke<VideoPropertiesDto | null>("get_video_properties", { videoId });
export const listSkippedFiles = () => invoke<SkippedFileDto[]>("list_skipped_files");
export const getFfmpegStatus = () => invoke<FfmpegStatusDto>("get_ffmpeg_status");
export const listWatchFolders = () => invoke<string[]>("list_watch_folders");
export const pickWatchFolders = () => invoke<string[]>("pick_watch_folders");
export const countVideosUnderFolder = (folderPath: string) =>
  invoke<number>("count_videos_under_folder", { folderPath });
export const removeWatchFolder = (folderPath: string) =>
  invoke<string[]>("remove_watch_folder", { folderPath });
export const renameWatchFolder = (oldFolderPath: string, newFolderPath: string) =>
  invoke<RenameWatchFolderResult>("rename_watch_folder", { oldFolderPath, newFolderPath });
export const startScan = () => invoke<ScanSummary>("start_scan");
export const toggleThumbnailPause = (paused: boolean) =>
  invoke<void>("toggle_thumbnail_pause", { paused });
export const getThumbnails = (videoId: string) =>
  invoke<string[] | null>("get_thumbnails", { videoId });
export const playVideo = (filePath: string) => invoke<void>("play_video", { filePath });
export const pickWbFile = () => invoke<string | null>("pick_wb_file");
export const pickWbThumbnailFolder = () => invoke<string | null>("pick_wb_thumbnail_folder");
export const startWbImport = (wbPath: string, thumbnailFolderPath: string) =>
  invoke<void>("start_wb_import", { wbPath, thumbnailFolderPath });
export const listDuplicateGroups = () => invoke<DuplicateGroup[]>("list_duplicate_groups");
export const refreshDuplicateGroups = () => invoke<void>("refresh_duplicate_groups");
export const deleteDuplicateVideo = (videoId: string) =>
  invoke<void>("delete_duplicate_video", { videoId });
export const listGenerationFailures = () =>
  invoke<GenerationFailuresDto>("list_generation_failures");
export const retryThumbnailGeneration = (videoId: string) =>
  invoke<void>("retry_thumbnail_generation", { videoId });
export const retryMetadataProbe = (videoId: string) =>
  invoke<void>("retry_metadata_probe", { videoId });
