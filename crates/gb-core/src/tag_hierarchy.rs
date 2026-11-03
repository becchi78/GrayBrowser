//! Two-level tag hierarchy: parent validation and display order. Pure logic
//! over a snapshot of the tags; the DB layer feeds it rows and applies the
//! outcome inside one transaction.
//!
//! A tag has at most one parent, a parent is itself top-level, and a tag with
//! children cannot get a parent. Together these make cycles and depth beyond
//! two levels unrepresentable, so no graph walk is needed.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// One tag as far as the hierarchy is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNode {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentError {
    /// The tag was given itself as its parent.
    SelfParent { tag_id: i64 },
    /// The chosen parent does not exist.
    ParentNotFound { parent_id: i64 },
    /// The chosen parent is itself a child of another tag.
    ParentIsChild { parent_id: i64 },
    /// The tag has children of its own, so it cannot become a child.
    HasChildren { tag_id: i64 },
}

impl fmt::Display for ParentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParentError::SelfParent { tag_id } => {
                write!(f, "tag {tag_id} cannot be its own parent")
            }
            ParentError::ParentNotFound { parent_id } => {
                write!(f, "parent tag {parent_id} does not exist")
            }
            ParentError::ParentIsChild { parent_id } => {
                write!(f, "tag {parent_id} is a child tag and cannot be a parent")
            }
            ParentError::HasChildren { tag_id } => {
                write!(f, "tag {tag_id} has child tags and cannot become a child")
            }
        }
    }
}

impl std::error::Error for ParentError {}

/// One entry of a saved layout: the tag and the parent it should end up with.
/// The position in the list is the display position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutItem {
    pub id: i64,
    pub parent_id: Option<i64>,
}

/// Checks that `tag_id` may have `new_parent`, judged against `tags` as they
/// would be after the change (so `tag_id`'s own current parent is irrelevant).
/// `None` always passes: becoming top-level cannot break the shape.
///
/// A `parent_id` pointing at a tag that no longer exists counts as top-level,
/// matching how it is displayed, so it neither blocks nor is blocked.
pub fn validate_parent(
    tag_id: i64,
    new_parent: Option<i64>,
    tags: &[TagNode],
) -> Result<(), ParentError> {
    let Some(parent_id) = new_parent else {
        return Ok(());
    };
    if parent_id == tag_id {
        return Err(ParentError::SelfParent { tag_id });
    }
    let by_id: HashMap<i64, &TagNode> = tags.iter().map(|t| (t.id, t)).collect();
    let Some(parent) = by_id.get(&parent_id) else {
        return Err(ParentError::ParentNotFound { parent_id });
    };
    if parent
        .parent_id
        .is_some_and(|grandparent| by_id.contains_key(&grandparent))
    {
        return Err(ParentError::ParentIsChild { parent_id });
    }
    if tags.iter().any(|t| t.parent_id == Some(tag_id)) {
        return Err(ParentError::HasChildren { tag_id });
    }
    Ok(())
}

/// Applies `items` on top of `current` and validates the result. Returns the
/// layout as it should be stored: items for tags that do not exist are
/// dropped (a tag deleted while the editor was open must not fail the save)
/// and a repeated id keeps its first position. Tags not listed keep their
/// current parent; the caller decides what happens to their order.
///
/// Every listed tag is checked against the final state, so a swap that only
/// works one step at a time (A under B while B is under nothing, then B under
/// A) is judged by what it ends up as, and a parent that is listed as a child
/// in the same save is rejected.
pub fn resolve_layout(
    current: &[TagNode],
    items: &[LayoutItem],
) -> Result<Vec<LayoutItem>, (i64, ParentError)> {
    let existing: HashSet<i64> = current.iter().map(|t| t.id).collect();
    let mut seen = HashSet::new();
    let resolved: Vec<LayoutItem> = items
        .iter()
        .copied()
        .filter(|item| existing.contains(&item.id) && seen.insert(item.id))
        .collect();

    let new_parent: HashMap<i64, Option<i64>> = resolved
        .iter()
        .map(|item| (item.id, item.parent_id))
        .collect();
    let after: Vec<TagNode> = current
        .iter()
        .map(|t| TagNode {
            parent_id: new_parent.get(&t.id).copied().unwrap_or(t.parent_id),
            ..t.clone()
        })
        .collect();

    for item in &resolved {
        validate_parent(item.id, item.parent_id, &after).map_err(|e| (item.id, e))?;
    }
    Ok(resolved)
}

/// Tags in display order, each with the parent it is shown under. Top-level
/// tags come by `sort_order` (unplaced ones last) then name, and each
/// parent's children follow it, ordered the same way. A tag whose parent is
/// missing, or is itself a child, is shown as top-level so that no tag
/// disappears from the list because of stale data.
pub fn display_order(tags: &[TagNode]) -> Vec<(i64, Option<i64>)> {
    let by_id: HashMap<i64, &TagNode> = tags.iter().map(|t| (t.id, t)).collect();
    let effective_parent = |t: &TagNode| -> Option<i64> {
        let parent = by_id.get(&t.parent_id?)?;
        (parent.id != t.id && parent.parent_id.is_none_or(|g| !by_id.contains_key(&g)))
            .then_some(parent.id)
    };

    let key = |t: &&TagNode| (t.sort_order.is_none(), t.sort_order, t.name.clone(), t.id);
    let mut top: Vec<&TagNode> = Vec::new();
    let mut children: HashMap<i64, Vec<&TagNode>> = HashMap::new();
    for t in tags {
        match effective_parent(t) {
            Some(p) => children.entry(p).or_default().push(t),
            None => top.push(t),
        }
    }
    top.sort_by_key(key);

    let mut out = Vec::with_capacity(tags.len());
    for t in top {
        out.push((t.id, None));
        if let Some(mut kids) = children.remove(&t.id) {
            kids.sort_by_key(key);
            out.extend(kids.into_iter().map(|k| (k.id, Some(t.id))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: i64, name: &str, parent_id: Option<i64>, sort_order: Option<i64>) -> TagNode {
        TagNode {
            id,
            name: name.to_string(),
            parent_id,
            sort_order,
        }
    }

    fn item(id: i64, parent_id: Option<i64>) -> LayoutItem {
        LayoutItem { id, parent_id }
    }

    // 1 and 2 are top-level; 3 is a child of 1.
    fn sample() -> Vec<TagNode> {
        vec![
            tag(1, "a", None, None),
            tag(2, "b", None, None),
            tag(3, "c", Some(1), None),
        ]
    }

    #[test]
    fn clearing_the_parent_always_passes() {
        assert_eq!(validate_parent(3, None, &sample()), Ok(()));
        assert_eq!(validate_parent(1, None, &sample()), Ok(()));
        assert_eq!(validate_parent(99, None, &[]), Ok(()));
    }

    #[test]
    fn a_top_level_tag_without_children_can_get_a_top_level_parent() {
        assert_eq!(validate_parent(2, Some(1), &sample()), Ok(()));
    }

    #[test]
    fn a_child_can_move_to_another_parent() {
        assert_eq!(validate_parent(3, Some(2), &sample()), Ok(()));
    }

    #[test]
    fn keeping_the_same_parent_passes() {
        assert_eq!(validate_parent(3, Some(1), &sample()), Ok(()));
    }

    #[test]
    fn rejects_itself_as_parent() {
        assert_eq!(
            validate_parent(2, Some(2), &sample()),
            Err(ParentError::SelfParent { tag_id: 2 })
        );
    }

    #[test]
    fn self_parent_is_reported_even_for_an_unknown_tag() {
        assert_eq!(
            validate_parent(99, Some(99), &sample()),
            Err(ParentError::SelfParent { tag_id: 99 })
        );
    }

    #[test]
    fn rejects_a_missing_parent() {
        assert_eq!(
            validate_parent(2, Some(99), &sample()),
            Err(ParentError::ParentNotFound { parent_id: 99 })
        );
    }

    #[test]
    fn rejects_a_parent_that_is_itself_a_child() {
        assert_eq!(
            validate_parent(2, Some(3), &sample()),
            Err(ParentError::ParentIsChild { parent_id: 3 })
        );
    }

    #[test]
    fn rejects_a_tag_that_has_children() {
        assert_eq!(
            validate_parent(1, Some(2), &sample()),
            Err(ParentError::HasChildren { tag_id: 1 })
        );
    }

    #[test]
    fn making_a_parent_a_child_of_its_own_child_is_rejected_as_a_child_parent() {
        // 1 -> 3 would be a cycle (3 is under 1). Rejected because 3 is a child.
        assert_eq!(
            validate_parent(1, Some(3), &sample()),
            Err(ParentError::ParentIsChild { parent_id: 3 })
        );
    }

    #[test]
    fn a_parent_whose_own_parent_is_missing_counts_as_top_level() {
        let tags = vec![tag(1, "a", Some(77), None), tag(2, "b", None, None)];
        assert_eq!(validate_parent(2, Some(1), &tags), Ok(()));
    }

    #[test]
    fn resolve_layout_accepts_a_valid_layout_and_keeps_its_order() {
        let items = [item(3, Some(2)), item(2, None), item(1, None)];
        assert_eq!(resolve_layout(&sample(), &items), Ok(items.to_vec()));
    }

    #[test]
    fn resolve_layout_drops_unknown_tags_and_repeated_ids() {
        let items = [
            item(99, None),
            item(2, Some(1)),
            item(2, None),
            item(1, None),
        ];
        assert_eq!(
            resolve_layout(&sample(), &items),
            Ok(vec![item(2, Some(1)), item(1, None)])
        );
    }

    #[test]
    fn resolve_layout_judges_the_final_state_not_each_step() {
        // 3 moves out from under 1 and 1 moves under 2 in the same save.
        let items = [item(3, None), item(1, Some(2)), item(2, None)];
        assert_eq!(resolve_layout(&sample(), &items), Ok(items.to_vec()));
    }

    #[test]
    fn resolve_layout_rejects_a_parent_listed_as_a_child_in_the_same_save() {
        let items = [item(1, Some(2)), item(2, Some(1))];
        let err = resolve_layout(&sample(), &items).unwrap_err();
        assert_eq!(err, (1, ParentError::ParentIsChild { parent_id: 2 }));
    }

    #[test]
    fn resolve_layout_rejects_a_child_under_an_unlisted_child() {
        let items = [item(2, Some(3))];
        assert_eq!(
            resolve_layout(&sample(), &items),
            Err((2, ParentError::ParentIsChild { parent_id: 3 }))
        );
    }

    #[test]
    fn resolve_layout_rejects_a_listed_parent_that_keeps_an_unlisted_child() {
        // 3 is not listed, so it stays under 1; 1 cannot go under 2.
        let items = [item(1, Some(2)), item(2, None)];
        assert_eq!(
            resolve_layout(&sample(), &items),
            Err((1, ParentError::HasChildren { tag_id: 1 }))
        );
    }

    #[test]
    fn resolve_layout_rejects_a_missing_parent_and_self_parent() {
        assert_eq!(
            resolve_layout(&sample(), &[item(2, Some(99))]),
            Err((2, ParentError::ParentNotFound { parent_id: 99 }))
        );
        assert_eq!(
            resolve_layout(&sample(), &[item(2, Some(2))]),
            Err((2, ParentError::SelfParent { tag_id: 2 }))
        );
    }

    #[test]
    fn resolve_layout_with_nothing_listed_is_empty() {
        assert_eq!(resolve_layout(&sample(), &[]), Ok(vec![]));
        assert_eq!(resolve_layout(&[], &[item(1, None)]), Ok(vec![]));
    }

    #[test]
    fn display_order_puts_children_right_after_their_parent() {
        let tags = vec![
            tag(1, "b", None, None),
            tag(2, "a", None, None),
            tag(3, "y", Some(1), None),
            tag(4, "x", Some(1), None),
            tag(5, "z", Some(2), None),
        ];
        assert_eq!(
            display_order(&tags),
            vec![
                (2, None),
                (5, Some(2)),
                (1, None),
                (4, Some(1)),
                (3, Some(1))
            ]
        );
    }

    #[test]
    fn display_order_sorts_by_sort_order_then_name_with_unplaced_last() {
        let tags = vec![
            tag(1, "a", None, None),
            tag(2, "b", None, Some(1)),
            tag(3, "c", None, Some(0)),
            tag(4, "d", Some(3), Some(5)),
            tag(5, "e", Some(3), Some(2)),
            tag(6, "f", Some(3), None),
        ];
        assert_eq!(
            display_order(&tags),
            vec![
                (3, None),
                (5, Some(3)),
                (4, Some(3)),
                (6, Some(3)),
                (2, None),
                (1, None)
            ]
        );
    }

    #[test]
    fn display_order_shows_a_child_of_a_missing_parent_as_top_level() {
        let tags = vec![tag(1, "a", Some(99), None), tag(2, "b", None, None)];
        assert_eq!(display_order(&tags), vec![(1, None), (2, None)]);
    }

    #[test]
    fn display_order_shows_a_third_level_tag_as_top_level() {
        // 3 hangs under 2, which hangs under 1: stale data, still listed once.
        let tags = vec![
            tag(1, "a", None, None),
            tag(2, "b", Some(1), None),
            tag(3, "c", Some(2), None),
        ];
        assert_eq!(
            display_order(&tags),
            vec![(1, None), (2, Some(1)), (3, None)]
        );
    }

    #[test]
    fn display_order_survives_a_self_parent_or_cycle() {
        let tags = vec![
            tag(1, "a", Some(1), None),
            tag(2, "b", Some(3), None),
            tag(3, "c", Some(2), None),
        ];
        let order = display_order(&tags);
        assert_eq!(order.len(), 3);
        let ids: HashSet<i64> = order.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, HashSet::from([1, 2, 3]));
    }

    #[test]
    fn display_order_after_the_parent_is_deleted_lists_the_children_at_top_level() {
        let mut tags = sample();
        tags.retain(|t| t.id != 1);
        assert_eq!(display_order(&tags), vec![(2, None), (3, None)]);
    }

    #[test]
    fn display_order_of_nothing_is_empty() {
        assert!(display_order(&[]).is_empty());
    }
}
