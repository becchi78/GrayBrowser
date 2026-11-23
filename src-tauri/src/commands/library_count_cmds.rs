//! Whole-library counts for the left pane and the list header. These ignore
//! the current filters on purpose: they answer "how many exist", while the
//! list itself answers "how many match".

use serde::Serialize;
use tauri::State;

use crate::db::{queries, Db};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FolderCountDto {
    pub folder_path: String,
    pub count: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RatingCountDto {
    pub rating: u8,
    pub count: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct LibraryCountsDto {
    pub total: u32,
    /// One entry per registered watch folder, in the folder list's order,
    /// including folders with no videos.
    pub folders: Vec<FolderCountDto>,
    /// Only ratings at least one video has; `rating` 0 is "unrated".
    pub ratings: Vec<RatingCountDto>,
}

fn library_counts(db: &Db) -> anyhow::Result<LibraryCountsDto> {
    let conn = db.read_pool.get()?;
    let total = queries::count_all_videos(&conn)?;
    let folders = queries::get_watch_folders(&conn)?
        .into_iter()
        .map(|folder_path| {
            let count = queries::count_videos_under_folder_on(&conn, &folder_path)?;
            Ok(FolderCountDto { folder_path, count })
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let ratings = queries::count_videos_by_rating(&conn)?
        .into_iter()
        .map(|(rating, count)| RatingCountDto { rating, count })
        .collect();
    Ok(LibraryCountsDto {
        total,
        folders,
        ratings,
    })
}

/// `async` so the aggregate queries over the whole library don't occupy the
/// main thread.
#[tauri::command(async)]
pub fn get_library_counts(db: State<Db>) -> Result<LibraryCountsDto, String> {
    library_counts(&db).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};

    fn set_rating(db: &Db, id: &str, rating: u8) {
        let conn = db.writer.lock().unwrap();
        queries::set_rating(&conn, id, rating).unwrap();
    }

    #[test]
    fn an_empty_library_has_zero_everything() {
        let (_d, db) = init_temp_db();
        assert_eq!(
            library_counts(&db).unwrap(),
            LibraryCountsDto {
                total: 0,
                folders: vec![],
                ratings: vec![]
            }
        );
    }

    #[test]
    fn counts_total_per_folder_and_per_rating() {
        let (_d, db) = init_temp_db();
        insert_test_video(&db, "a", r"C:\Videos\a.mp4");
        insert_test_video(&db, "b", r"C:\Videos\sub\b.mp4");
        // Same string prefix as "C:\Videos" but a different folder.
        insert_test_video(&db, "c", r"C:\Videos2\c.mp4");
        insert_test_video(&db, "d", r"D:\Movies\d.mp4");
        set_rating(&db, "a", 5);
        set_rating(&db, "b", 5);
        set_rating(&db, "d", 3);
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(
                &conn,
                &[
                    r"C:\Videos".to_string(),
                    r"D:\Movies".to_string(),
                    r"E:\Empty".to_string(),
                ],
            )
            .unwrap();
        }

        let counts = library_counts(&db).unwrap();

        assert_eq!(counts.total, 4);
        let folders: Vec<(&str, u32)> = counts
            .folders
            .iter()
            .map(|f| (f.folder_path.as_str(), f.count))
            .collect();
        assert_eq!(
            folders,
            vec![(r"C:\Videos", 2), (r"D:\Movies", 1), (r"E:\Empty", 0)]
        );
        let ratings: Vec<(u8, u32)> = counts.ratings.iter().map(|r| (r.rating, r.count)).collect();
        assert_eq!(ratings, vec![(0, 1), (3, 1), (5, 2)]);
    }

    #[test]
    fn offline_videos_are_counted_like_the_list_shows_them() {
        let (_d, db) = init_temp_db();
        insert_test_video(&db, "a", r"C:\Videos\a.mp4");
        {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
        }
        assert_eq!(library_counts(&db).unwrap().total, 1);
    }
}
