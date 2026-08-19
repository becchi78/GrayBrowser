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
}
