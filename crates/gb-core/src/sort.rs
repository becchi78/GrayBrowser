//! Sort-order resolution. `rusqlite` can't bind column identifiers
//! as query parameters, so the `ORDER BY` clause is built via string
//! interpolation in `src-tauri::db::queries` -- this module is the *only*
//! thing allowed to produce that string, from a closed enum, so no raw
//! frontend string ever reaches SQL text directly.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    FileName,
    CreatedAt,
    /// UI label is "更新日", but the underlying value is `videos.mtime` --
    /// the filesystem modification time, NOT a "last catalog edit"/"整理日"
    /// timestamp: tagging/rating edits deliberately do not bump this value,
    /// only rescans/reconciliation do -- there is no "last organized"
    /// concept at all.
    UpdatedDate,
    Rating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// Returns the `ORDER BY` clause body (everything after the `ORDER BY`
/// keyword) for a given field/direction. Written as an exhaustive `match`
/// with no wildcard arm, so adding a new `SortField` variant fails to
/// compile here until this function is updated to handle it.
///
/// `UpdatedDate` pins NULL `mtime` rows to the end regardless of direction:
/// rows that predate mtime tracking or were never rescanned/reconciled are
/// left with `mtime` NULL -- an unset value, not "least recent" -- so it
/// must not interleave with real timestamps at either end of the sort.
/// (No index backs this compound expression -- verified via
/// `EXPLAIN QUERY PLAN` in `src-tauri/tests/sort_index_usage.rs` -- a known,
/// accepted cost for correct NULL placement.)
pub fn order_by_clause(field: SortField, direction: SortDirection) -> String {
    let dir = match direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    };
    match field {
        SortField::FileName => format!("file_name {dir}"),
        SortField::CreatedAt => format!("created_at {dir}"),
        SortField::Rating => format!("rating {dir}"),
        SortField::UpdatedDate => format!("mtime IS NULL, mtime {dir}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_asc_and_desc() {
        assert_eq!(
            order_by_clause(SortField::FileName, SortDirection::Asc),
            "file_name ASC"
        );
        assert_eq!(
            order_by_clause(SortField::FileName, SortDirection::Desc),
            "file_name DESC"
        );
    }

    #[test]
    fn created_at_asc_and_desc() {
        assert_eq!(
            order_by_clause(SortField::CreatedAt, SortDirection::Asc),
            "created_at ASC"
        );
        assert_eq!(
            order_by_clause(SortField::CreatedAt, SortDirection::Desc),
            "created_at DESC"
        );
    }

    #[test]
    fn rating_asc_and_desc() {
        assert_eq!(
            order_by_clause(SortField::Rating, SortDirection::Asc),
            "rating ASC"
        );
        assert_eq!(
            order_by_clause(SortField::Rating, SortDirection::Desc),
            "rating DESC"
        );
    }

    #[test]
    fn updated_date_pins_nulls_last_in_both_directions() {
        assert_eq!(
            order_by_clause(SortField::UpdatedDate, SortDirection::Asc),
            "mtime IS NULL, mtime ASC"
        );
        assert_eq!(
            order_by_clause(SortField::UpdatedDate, SortDirection::Desc),
            "mtime IS NULL, mtime DESC"
        );
    }
}
