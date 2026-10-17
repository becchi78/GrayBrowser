//! Decides whether a catalogued video's source file may be moved to the
//! recycle bin. Pure: callers gather the facts (DB row, drive type, file
//! metadata) and this function only judges them.
//!
//! The policy is deny-by-default: every check must pass, and each input is
//! shaped so that "unknown" or "could not tell" maps to a refusal rather
//! than to permission.

use crate::ports::drive_type::DriveKind;

/// What the caller found on disk for the DB's `file_path`. Anything the
/// caller could not positively establish as a regular file (stat failure,
/// permission error, symlink, directory) must be reported as a non-`Regular`
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Missing,
    NotRegularFile,
    Regular { size: u64 },
}

/// Whether the group's members are known to have identical content. File
/// size and quick hash matching alone do not prove a file is a copy, so only
/// a content-verified group may lead to deleting a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupContent {
    Confirmed,
    Unconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDeleteFacts<'a> {
    /// The `videos.status` value; only `"online"` passes.
    pub status: &'a str,
    pub drive: DriveKind,
    pub group_content: GroupContent,
    /// Online members of the video's duplicate group, including the target
    /// itself.
    pub online_members_in_group: usize,
    pub under_registered_folder: bool,
    pub file: FileState,
    /// `videos.file_size` as recorded at registration.
    pub recorded_size: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeleteRefusal {
    #[error("オフラインの動画のファイルは削除できません")]
    Offline,
    #[error("ゴミ箱に移動できない場所です。エクスプローラーで直接削除してください")]
    TrashUnavailable,
    #[error("内容の一致が確認できていないグループのため、ファイルは削除できません")]
    ContentNotConfirmed,
    #[error("グループ内で到達できる最後の1件のため、ファイルは削除できません")]
    LastOnlineInGroup,
    #[error("登録フォルダの外にあるファイルは削除できません")]
    OutsideRegisteredFolders,
    #[error("ファイルが見つからないか、通常のファイルではありません")]
    FileMissingOrNotRegular,
    #[error("ファイルのサイズが登録時と異なります。再スキャンしてください")]
    SizeMismatch,
}

pub fn check_file_delete_allowed(facts: &FileDeleteFacts) -> Result<(), DeleteRefusal> {
    if facts.status != "online" {
        return Err(DeleteRefusal::Offline);
    }
    // Only fixed local drives are known to have a recycle bin; on the rest
    // the OS may permanently delete without telling us.
    if facts.drive != DriveKind::Local {
        return Err(DeleteRefusal::TrashUnavailable);
    }
    if facts.group_content != GroupContent::Confirmed {
        return Err(DeleteRefusal::ContentNotConfirmed);
    }
    if facts.online_members_in_group <= 1 {
        return Err(DeleteRefusal::LastOnlineInGroup);
    }
    if !facts.under_registered_folder {
        return Err(DeleteRefusal::OutsideRegisteredFolders);
    }
    let actual = match facts.file {
        FileState::Regular { size } => size,
        FileState::Missing | FileState::NotRegularFile => {
            return Err(DeleteRefusal::FileMissingOrNotRegular)
        }
    };
    if u64::try_from(facts.recorded_size) != Ok(actual) {
        return Err(DeleteRefusal::SizeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_facts() -> FileDeleteFacts<'static> {
        FileDeleteFacts {
            status: "online",
            drive: DriveKind::Local,
            group_content: GroupContent::Confirmed,
            online_members_in_group: 2,
            under_registered_folder: true,
            file: FileState::Regular { size: 100 },
            recorded_size: 100,
        }
    }

    #[test]
    fn allows_when_every_check_passes() {
        assert_eq!(check_file_delete_allowed(&ok_facts()), Ok(()));
    }

    #[test]
    fn allows_with_more_than_two_online_members() {
        let f = FileDeleteFacts {
            online_members_in_group: 5,
            ..ok_facts()
        };
        assert_eq!(check_file_delete_allowed(&f), Ok(()));
    }

    #[test]
    fn refuses_offline_and_any_non_online_status() {
        for status in ["offline", "", "ONLINE", "unknown"] {
            let f = FileDeleteFacts {
                status,
                ..ok_facts()
            };
            assert_eq!(
                check_file_delete_allowed(&f),
                Err(DeleteRefusal::Offline),
                "{status}"
            );
        }
    }

    #[test]
    fn refuses_every_non_local_drive() {
        for drive in [DriveKind::Network, DriveKind::Removable, DriveKind::Unknown] {
            let f = FileDeleteFacts {
                drive,
                ..ok_facts()
            };
            assert_eq!(
                check_file_delete_allowed(&f),
                Err(DeleteRefusal::TrashUnavailable),
                "{drive:?}"
            );
        }
    }

    #[test]
    fn refuses_a_group_whose_content_is_not_confirmed() {
        let f = FileDeleteFacts {
            group_content: GroupContent::Unconfirmed,
            ..ok_facts()
        };
        assert_eq!(
            check_file_delete_allowed(&f),
            Err(DeleteRefusal::ContentNotConfirmed)
        );
    }

    #[test]
    fn refuses_the_last_online_member() {
        for n in [0, 1] {
            let f = FileDeleteFacts {
                online_members_in_group: n,
                ..ok_facts()
            };
            assert_eq!(
                check_file_delete_allowed(&f),
                Err(DeleteRefusal::LastOnlineInGroup),
                "{n}"
            );
        }
    }

    #[test]
    fn refuses_outside_registered_folders() {
        let f = FileDeleteFacts {
            under_registered_folder: false,
            ..ok_facts()
        };
        assert_eq!(
            check_file_delete_allowed(&f),
            Err(DeleteRefusal::OutsideRegisteredFolders)
        );
    }

    #[test]
    fn refuses_missing_or_non_regular_files() {
        for file in [FileState::Missing, FileState::NotRegularFile] {
            let f = FileDeleteFacts { file, ..ok_facts() };
            assert_eq!(
                check_file_delete_allowed(&f),
                Err(DeleteRefusal::FileMissingOrNotRegular)
            );
        }
    }

    #[test]
    fn refuses_size_mismatch_including_negative_recorded_size() {
        for (recorded_size, size) in [(100, 101), (100, 99), (-1, 0), (-100, 100), (0, 1)] {
            let f = FileDeleteFacts {
                file: FileState::Regular { size },
                recorded_size,
                ..ok_facts()
            };
            assert_eq!(
                check_file_delete_allowed(&f),
                Err(DeleteRefusal::SizeMismatch),
                "{recorded_size} vs {size}"
            );
        }
    }

    #[test]
    fn allows_an_empty_file_whose_recorded_size_is_also_zero() {
        let f = FileDeleteFacts {
            file: FileState::Regular { size: 0 },
            recorded_size: 0,
            ..ok_facts()
        };
        assert_eq!(check_file_delete_allowed(&f), Ok(()));
    }

    #[test]
    fn earlier_checks_take_priority_over_later_ones() {
        let f = FileDeleteFacts {
            status: "offline",
            drive: DriveKind::Network,
            group_content: GroupContent::Unconfirmed,
            online_members_in_group: 0,
            under_registered_folder: false,
            file: FileState::Missing,
            recorded_size: -1,
        };
        assert_eq!(check_file_delete_allowed(&f), Err(DeleteRefusal::Offline));
    }
}
