//! Tests of the `.wb` import pipeline, one file per topic. Most run a whole
//! import through `import_all` on a temporary database and fake adapters.

use super::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use gb_core::ports::ffmpeg::FfmpegError;
use gb_core::ports::ffmpeg::{CropRect, VideoMetadata};
use gb_core::ports::volume_label::LabelResolution;
use gb_core::ports::wb_source::WbMovieRow;
use gb_core::ports::wb_source::WbSourceError;
use gb_core::testing::fake_dir_lister::FakeDirLister;
use gb_core::testing::fake_ffmpeg::FakeCall;
use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;
use gb_core::testing::fake_folder_probe::FakeFolderProbe;
use gb_core::testing::fake_volume_label::FakeVolumeLabelResolver;
use gb_core::testing::fake_wb_import_clock::FakeWbImportClock;
use gb_core::testing::fake_wb_import_log::FakeWbImportLog;
use gb_core::testing::fake_wb_source::FakeWbSourceAdapter;
use gb_core::wb_import::{extract_thumbnail_hash, file_name_from_path, match_thumbnail_files};
use gb_core::wb_thumbnail_split::ThumbLayout;

use crate::db::test_support::init_temp_db;
use crate::events::{
    FakeCatalogNotifier, FakeWatchReconfigurer, FakeWbImportNotifier, WbImportEvent,
};
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;
use crate::wb_import::thumbnail_scan;

use super::thumbnails::LinkOutcome;

mod common;
mod file_names;
mod general;
mod import_log;
mod name_link;
mod path_restore;
mod progress;
mod thumbnail_link;
mod thumbnail_split;
mod unreachable_shares;
mod volume_labels;
mod watch_folders;
