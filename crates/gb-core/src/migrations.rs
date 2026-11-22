//! Pure migration-ordering logic. SQL execution against a real
//! `rusqlite::Connection` is the `src-tauri::db::migrations` adapter's
//! responsibility, not this crate's.

pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
}

/// Returns the migrations with `version > current_version`, sorted ascending
/// by version regardless of the input order.
pub fn pending_migrations(current_version: i64, all: &[Migration]) -> Vec<&Migration> {
    let mut pending: Vec<&Migration> = all.iter().filter(|m| m.version > current_version).collect();
    pending.sort_by_key(|m| m.version);
    pending
}

/// The database was written by a newer app than this one. Migrations cannot be
/// rolled back, so opening it anyway could corrupt data this app does not
/// understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("database schema version {db_version} is newer than this app supports ({app_version})")]
pub struct SchemaTooNew {
    pub db_version: i64,
    pub app_version: i64,
}

/// Fails only when `db_version` is above `latest_known`. 0 (an empty
/// database), an equal version and an older one are all fine.
pub fn check_schema_compat(db_version: i64, latest_known: i64) -> Result<(), SchemaTooNew> {
    if db_version > latest_known {
        return Err(SchemaTooNew {
            db_version,
            app_version: latest_known,
        });
    }
    Ok(())
}

/// Highest version in `all`, or 0 for an empty list.
pub fn latest_version(all: &[Migration]) -> i64 {
    all.iter().map(|m| m.version).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: Migration = Migration {
        version: 1,
        description: "v1",
        sql: "-- v1",
    };
    const V2: Migration = Migration {
        version: 2,
        description: "v2",
        sql: "-- v2",
    };
    const V3: Migration = Migration {
        version: 3,
        description: "v3",
        sql: "-- v3",
    };

    #[test]
    fn returns_all_migrations_in_ascending_order_when_current_is_zero() {
        let all = [V1, V2, V3];
        let pending = pending_migrations(0, &all);
        let versions: Vec<i64> = pending.iter().map(|m| m.version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[test]
    fn returns_only_migrations_newer_than_current() {
        let all = [V1, V2, V3];
        let pending = pending_migrations(2, &all);
        let versions: Vec<i64> = pending.iter().map(|m| m.version).collect();
        assert_eq!(versions, vec![3]);
    }

    #[test]
    fn returns_empty_when_current_is_at_or_above_the_max_version() {
        let all = [V1, V2, V3];
        assert!(pending_migrations(3, &all).is_empty());
        assert!(pending_migrations(10, &all).is_empty());
    }

    #[test]
    fn sorts_unsorted_input_by_version() {
        let all = [V3, V1, V2];
        let pending = pending_migrations(0, &all);
        let versions: Vec<i64> = pending.iter().map(|m| m.version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[test]
    fn returns_empty_for_an_empty_migration_list() {
        let all: [Migration; 0] = [];
        assert!(pending_migrations(0, &all).is_empty());
    }

    #[test]
    fn compat_rejects_only_a_newer_database() {
        assert_eq!(
            check_schema_compat(14, 13),
            Err(SchemaTooNew {
                db_version: 14,
                app_version: 13
            })
        );
        assert!(check_schema_compat(13, 13).is_ok());
        assert!(check_schema_compat(12, 13).is_ok());
        assert!(check_schema_compat(0, 13).is_ok());
        assert!(check_schema_compat(1, 0).is_err());
    }

    #[test]
    fn latest_version_is_the_max_regardless_of_order() {
        assert_eq!(latest_version(&[V3, V1, V2]), 3);
        assert_eq!(latest_version(&[V1]), 1);
        assert_eq!(latest_version(&[]), 0);
    }
}
