use std::path::Path;

use gb_core::ports::file_revealer::FileRevealer;
use tauri::State;

use crate::adapters;
use crate::db::{queries, Db};

/// A `::label\...` path means the owning folder could not be resolved to a
/// real location, so there is nothing the shell could open.
fn reveal_video(revealer: &impl FileRevealer, file_path: &str) -> Result<(), String> {
    if file_path.trim().is_empty() {
        return Err("動画のパスが登録されていません".to_string());
    }
    if file_path.starts_with("::") {
        return Err("パスを解決できない動画のため、エクスプローラーで開けません".to_string());
    }
    revealer
        .reveal(Path::new(file_path))
        .map_err(|e| e.to_string())
}

/// Resolves the registered path from the video id (the caller never supplies
/// a path) and reveals it. `lookup` is injected so tests need no database.
fn reveal_registered_video(
    revealer: &impl FileRevealer,
    video_id: &str,
    lookup: impl FnOnce(&str) -> anyhow::Result<Option<String>>,
) -> Result<(), String> {
    let file_path = lookup(video_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("動画が見つかりません: {video_id}"))?;
    reveal_video(revealer, &file_path)
}

/// Runs on a blocking worker: the shell call can stall on a slow network
/// path and must not occupy the UI thread.
#[tauri::command]
pub async fn reveal_video_in_explorer(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    video_id: String,
) -> Result<(), String> {
    let pool = db.read_pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let revealer = adapters::file_revealer::RealFileRevealer::new(app);
        let result = reveal_registered_video(&revealer, &video_id, |id| {
            Ok(queries::find_video_by_id(&pool, id)?.map(|row| row.file_path))
        });
        if let Err(e) = &result {
            log::warn!("failed to reveal video {video_id} in explorer: {e}");
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::file_revealer::RevealError;
    use gb_core::testing::fake_file_revealer::FakeFileRevealer;
    use std::path::PathBuf;

    fn ok_fake() -> FakeFileRevealer {
        FakeFileRevealer {
            result: Ok(()),
            ..Default::default()
        }
    }

    #[test]
    fn registered_path_is_revealed() {
        let fake = ok_fake();
        let r = reveal_registered_video(&fake, "v1", |id| {
            assert_eq!(id, "v1");
            Ok(Some(r"D:\動画 一覧\a.mp4".to_string()))
        });
        assert!(r.is_ok());
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![PathBuf::from(r"D:\動画 一覧\a.mp4")]
        );
    }

    #[test]
    fn reveal_failure_is_propagated() {
        let fake = FakeFileRevealer {
            result: Err(RevealError::Reveal("not found".into())),
            ..Default::default()
        };
        let err = reveal_registered_video(&fake, "v1", |_| Ok(Some("a.mp4".into()))).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn unknown_video_id_fails_without_revealing() {
        let fake = ok_fake();
        let err = reveal_registered_video(&fake, "nope", |_| Ok(None)).unwrap_err();
        assert!(err.contains("nope"));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn lookup_error_is_propagated_without_revealing() {
        let fake = ok_fake();
        let err =
            reveal_registered_video(&fake, "v1", |_| Err(anyhow::anyhow!("db down"))).unwrap_err();
        assert!(err.contains("db down"));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn empty_path_fails_without_revealing() {
        let fake = ok_fake();
        assert!(reveal_video(&fake, "").is_err());
        assert!(reveal_video(&fake, "  ").is_err());
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn unresolved_label_path_fails_without_revealing() {
        let fake = ok_fake();
        assert!(reveal_video(&fake, r"::NAS\a.mp4").is_err());
        assert!(fake.calls.lock().unwrap().is_empty());
    }
}
