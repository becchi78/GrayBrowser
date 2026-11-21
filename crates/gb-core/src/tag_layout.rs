//! Display layout of tags: a single top-level column mixing ungrouped tags and
//! groups (each holding a column of tags). Pure data in, pure data out, so the
//! move/renumber rules can be tested without a database.
//!
//! Persisted order is a pair of `sort_order` numbers: ungrouped tags and groups
//! share one number line for the top level, and tags inside a group have their
//! own line. Every edit renumbers the whole layout (see `assignments`) so that
//! a `NULL`/number mix or a stale number can never leave two items tied.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub id: i64,
    pub name: String,
    pub tags: Vec<TagEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutItem {
    Tag(TagEntry),
    Group(GroupEntry),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagLayout {
    pub items: Vec<LayoutItem>,
}

/// What a move or removal refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemRef {
    Tag(i64),
    Group(i64),
}

impl ItemRef {
    /// Parses the `kind` string the UI sends; `None` for anything unknown.
    pub fn parse(kind: &str, id: i64) -> Option<Self> {
        match kind {
            "tag" => Some(ItemRef::Tag(id)),
            "group" => Some(ItemRef::Group(id)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "up" => Some(Direction::Up),
            "down" => Some(Direction::Down),
            _ => None,
        }
    }
}

/// Row to store for a tag: its group (`None` = top level) and position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagAssignment {
    pub id: i64,
    pub group_id: Option<i64>,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAssignment {
    pub id: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Assignments {
    pub tags: Vec<TagAssignment>,
    pub groups: Vec<GroupAssignment>,
}

type TagRowRef<'a> = &'a (i64, String, Option<i64>, Option<i64>);
type SortKey = (bool, i64, String, i64);

/// Sort key: placed items (`Some`) first by number, unplaced (`None`) after,
/// then by name, then id so the result is total and deterministic.
fn key(order: Option<i64>, name: &str, id: i64) -> SortKey {
    (order.is_none(), order.unwrap_or(0), name.to_string(), id)
}

/// Builds the layout from stored rows. `groups` are `(id, name, sort_order)`;
/// `tags` are `(id, name, sort_order, group_id)`. A tag whose `group_id` names
/// no existing group is treated as top level: with foreign keys off, a stale
/// `group_id` must degrade to "ungrouped" rather than hide the tag.
pub fn build_layout(
    groups: &[(i64, String, Option<i64>)],
    tags: &[(i64, String, Option<i64>, Option<i64>)],
) -> TagLayout {
    let group_ids: HashSet<i64> = groups.iter().map(|g| g.0).collect();

    let mut members: HashMap<i64, Vec<TagRowRef>> = HashMap::new();
    let mut top: Vec<(SortKey, u8, LayoutItem)> = Vec::new();
    for t in tags {
        match t.3.filter(|g| group_ids.contains(g)) {
            Some(g) => members.entry(g).or_default().push(t),
            None => top.push((
                key(t.2, &t.1, t.0),
                0,
                LayoutItem::Tag(TagEntry {
                    id: t.0,
                    name: t.1.clone(),
                }),
            )),
        }
    }
    for (id, name, order) in groups {
        let mut inner = members.remove(id).unwrap_or_default();
        inner.sort_by_key(|t| key(t.2, &t.1, t.0));
        top.push((
            key(*order, name, *id),
            1,
            LayoutItem::Group(GroupEntry {
                id: *id,
                name: name.clone(),
                tags: inner
                    .into_iter()
                    .map(|t| TagEntry {
                        id: t.0,
                        name: t.1.clone(),
                    })
                    .collect(),
            }),
        ));
    }
    // The kind byte only breaks a full tie between a tag and a group.
    top.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    TagLayout {
        items: top.into_iter().map(|(_, _, item)| item).collect(),
    }
}

fn is_tag(item: &LayoutItem, id: i64) -> bool {
    matches!(item, LayoutItem::Tag(t) if t.id == id)
}

/// Moves one item one step in the visible order. `None` when it cannot move
/// (already at the top/bottom edge, or not in the layout).
pub fn move_item(layout: &TagLayout, item: ItemRef, direction: Direction) -> Option<TagLayout> {
    let mut items = layout.items.clone();
    match item {
        ItemRef::Group(id) => {
            let i = items
                .iter()
                .position(|x| matches!(x, LayoutItem::Group(g) if g.id == id))?;
            let j = step(i, direction, items.len())?;
            items.swap(i, j);
        }
        ItemRef::Tag(id) => {
            if let Some(i) = items.iter().position(|x| is_tag(x, id)) {
                let j = step(i, direction, items.len())?;
                match direction {
                    Direction::Up => match &items[j] {
                        LayoutItem::Tag(_) => items.swap(i, j),
                        LayoutItem::Group(_) => {
                            let LayoutItem::Tag(t) = items.remove(i) else {
                                unreachable!()
                            };
                            // j < i, so removing i leaves the group at j.
                            if let LayoutItem::Group(g) = &mut items[j] {
                                g.tags.push(t);
                            }
                        }
                    },
                    Direction::Down => match &items[j] {
                        LayoutItem::Tag(_) => items.swap(i, j),
                        LayoutItem::Group(_) => {
                            let LayoutItem::Tag(t) = items.remove(i) else {
                                unreachable!()
                            };
                            // The group shifted from j to i after the removal.
                            if let LayoutItem::Group(g) = &mut items[i] {
                                g.tags.insert(0, t);
                            }
                        }
                    },
                }
            } else {
                let gi = items.iter().position(
                    |x| matches!(x, LayoutItem::Group(g) if g.tags.iter().any(|t| t.id == id)),
                )?;
                let LayoutItem::Group(g) = &mut items[gi] else {
                    unreachable!()
                };
                let ti = g.tags.iter().position(|t| t.id == id)?;
                match direction {
                    Direction::Up if ti > 0 => g.tags.swap(ti, ti - 1),
                    Direction::Down if ti + 1 < g.tags.len() => g.tags.swap(ti, ti + 1),
                    Direction::Up => {
                        let t = g.tags.remove(ti);
                        items.insert(gi, LayoutItem::Tag(t));
                    }
                    Direction::Down => {
                        let t = g.tags.remove(ti);
                        items.insert(gi + 1, LayoutItem::Tag(t));
                    }
                }
            }
        }
    }
    Some(TagLayout { items })
}

fn step(i: usize, direction: Direction, len: usize) -> Option<usize> {
    match direction {
        Direction::Up => i.checked_sub(1),
        Direction::Down => (i + 1 < len).then_some(i + 1),
    }
}

/// Dissolves a group in place: its tags become top-level tags at the group's
/// position, in their group order. Unchanged when the group does not exist.
pub fn remove_group(layout: &TagLayout, group_id: i64) -> TagLayout {
    let mut items = Vec::with_capacity(layout.items.len());
    for item in &layout.items {
        match item {
            LayoutItem::Group(g) if g.id == group_id => {
                items.extend(g.tags.iter().cloned().map(LayoutItem::Tag));
            }
            other => items.push(other.clone()),
        }
    }
    TagLayout { items }
}

/// Puts a new empty group first.
pub fn insert_group_at_top(layout: &TagLayout, group_id: i64, name: &str) -> TagLayout {
    let mut items = Vec::with_capacity(layout.items.len() + 1);
    items.push(LayoutItem::Group(GroupEntry {
        id: group_id,
        name: name.to_string(),
        tags: Vec::new(),
    }));
    items.extend(layout.items.iter().cloned());
    TagLayout { items }
}

/// Numbers the whole layout: the top level 0.., each group's tags 0.. again.
pub fn assignments(layout: &TagLayout) -> Assignments {
    let mut out = Assignments::default();
    for (n, item) in layout.items.iter().enumerate() {
        let n = n as i64;
        match item {
            LayoutItem::Tag(t) => out.tags.push(TagAssignment {
                id: t.id,
                group_id: None,
                sort_order: n,
            }),
            LayoutItem::Group(g) => {
                out.groups.push(GroupAssignment {
                    id: g.id,
                    sort_order: n,
                });
                for (m, t) in g.tags.iter().enumerate() {
                    out.tags.push(TagAssignment {
                        id: t.id,
                        group_id: Some(g.id),
                        sort_order: m as i64,
                    });
                }
            }
        }
    }
    out
}

/// Tags in display order with groups expanded in place, for flat pickers.
pub fn flatten(layout: &TagLayout) -> Vec<&TagEntry> {
    let mut out = Vec::new();
    for item in &layout.items {
        match item {
            LayoutItem::Tag(t) => out.push(t),
            LayoutItem::Group(g) => out.extend(g.tags.iter()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::{Down, Up};

    /// Compact notation: "a" is a top-level tag, "G1[b,c]" a group with id 1.
    /// Tag ids are the char code of the (single-char) name; group id is the digit.
    fn show(layout: &TagLayout) -> String {
        layout
            .items
            .iter()
            .map(|i| match i {
                LayoutItem::Tag(t) => t.name.clone(),
                LayoutItem::Group(g) => format!(
                    "G{}[{}]",
                    g.id,
                    g.tags
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn tag(name: &str) -> TagEntry {
        TagEntry {
            id: name.chars().next().unwrap() as i64,
            name: name.to_string(),
        }
    }

    fn t(name: &str) -> LayoutItem {
        LayoutItem::Tag(tag(name))
    }

    fn g(id: i64, names: &[&str]) -> LayoutItem {
        LayoutItem::Group(GroupEntry {
            id,
            name: format!("grp{id}"),
            tags: names.iter().map(|n| tag(n)).collect(),
        })
    }

    fn layout(items: Vec<LayoutItem>) -> TagLayout {
        TagLayout { items }
    }

    fn tid(name: &str) -> ItemRef {
        ItemRef::Tag(tag(name).id)
    }

    fn mv(l: &TagLayout, item: ItemRef, d: Direction) -> Option<String> {
        move_item(l, item, d).map(|l| show(&l))
    }

    fn sorted_tag_ids(l: &TagLayout) -> Vec<i64> {
        let mut v: Vec<i64> = flatten(l).iter().map(|t| t.id).collect();
        v.sort();
        v
    }

    fn sample() -> TagLayout {
        // a G1[b,c] d G2[] e
        layout(vec![t("a"), g(1, &["b", "c"]), t("d"), g(2, &[]), t("e")])
    }

    #[test]
    fn top_level_tag_swaps_with_adjacent_tag() {
        let l = layout(vec![t("a"), t("b"), t("c")]);
        assert_eq!(mv(&l, tid("b"), Up).unwrap(), "b a c");
        assert_eq!(mv(&l, tid("b"), Down).unwrap(), "a c b");
    }

    #[test]
    fn top_level_tag_at_the_edges_cannot_move() {
        let l = layout(vec![t("a"), t("b")]);
        assert_eq!(mv(&l, tid("a"), Up), None);
        assert_eq!(mv(&l, tid("b"), Down), None);
    }

    #[test]
    fn tag_moving_up_enters_the_group_above_at_its_end() {
        let l = sample();
        assert_eq!(mv(&l, tid("d"), Up).unwrap(), "a G1[b,c,d] G2[] e");
    }

    #[test]
    fn tag_moving_down_enters_the_group_below_at_its_start() {
        let l = sample();
        assert_eq!(mv(&l, tid("a"), Down).unwrap(), "G1[a,b,c] d G2[] e");
    }

    #[test]
    fn tag_can_enter_an_empty_group_from_both_sides() {
        let l = sample();
        assert_eq!(mv(&l, tid("e"), Up).unwrap(), "a G1[b,c] d G2[e]");
        let l = layout(vec![g(2, &[]), t("e")]);
        assert_eq!(mv(&l, tid("e"), Up).unwrap(), "G2[e]");
        let l = layout(vec![t("e"), g(2, &[])]);
        assert_eq!(mv(&l, tid("e"), Down).unwrap(), "G2[e]");
    }

    #[test]
    fn tag_in_group_swaps_inside_and_leaves_at_the_edges() {
        let l = layout(vec![t("a"), g(1, &["b", "c", "d"]), t("e")]);
        assert_eq!(mv(&l, tid("c"), Up).unwrap(), "a G1[c,b,d] e");
        assert_eq!(mv(&l, tid("c"), Down).unwrap(), "a G1[b,d,c] e");
        // first member up: out above the group
        assert_eq!(mv(&l, tid("b"), Up).unwrap(), "a b G1[c,d] e");
        // last member down: out below the group
        assert_eq!(mv(&l, tid("d"), Down).unwrap(), "a G1[b,c] d e");
    }

    #[test]
    fn leaving_a_group_at_the_list_edge_still_works() {
        let l = layout(vec![g(1, &["a", "b"])]);
        assert_eq!(mv(&l, tid("a"), Up).unwrap(), "a G1[b]");
        assert_eq!(mv(&l, tid("b"), Down).unwrap(), "G1[a] b");
    }

    #[test]
    fn group_swaps_with_neighbours_carrying_its_tags() {
        let l = sample();
        assert_eq!(mv(&l, ItemRef::Group(1), Up).unwrap(), "G1[b,c] a d G2[] e");
        assert_eq!(
            mv(&l, ItemRef::Group(1), Down).unwrap(),
            "a d G1[b,c] G2[] e"
        );
        assert_eq!(
            mv(&l, ItemRef::Group(2), Down).unwrap(),
            "a G1[b,c] d e G2[]"
        );
        let two = layout(vec![g(1, &["b"]), g(2, &[])]);
        assert_eq!(mv(&two, ItemRef::Group(1), Down).unwrap(), "G2[] G1[b]");
    }

    #[test]
    fn group_at_the_edges_cannot_move() {
        let l = layout(vec![g(1, &["a"]), t("b"), g(2, &[])]);
        assert_eq!(mv(&l, ItemRef::Group(1), Up), None);
        assert_eq!(mv(&l, ItemRef::Group(2), Down), None);
    }

    #[test]
    fn unknown_items_cannot_move() {
        let l = sample();
        assert_eq!(mv(&l, ItemRef::Tag(9999), Up), None);
        assert_eq!(mv(&l, ItemRef::Group(9999), Down), None);
        assert_eq!(move_item(&TagLayout::default(), tid("a"), Up), None);
    }

    #[test]
    fn up_then_down_restores_the_layout_for_every_item() {
        let l = sample();
        let mut refs: Vec<ItemRef> = flatten(&l).iter().map(|t| ItemRef::Tag(t.id)).collect();
        refs.push(ItemRef::Group(1));
        refs.push(ItemRef::Group(2));
        for r in refs {
            if let Some(up) = move_item(&l, r, Up) {
                assert_eq!(move_item(&up, r, Down).as_ref(), Some(&l), "{r:?} up/down");
            }
            if let Some(down) = move_item(&l, r, Down) {
                assert_eq!(move_item(&down, r, Up).as_ref(), Some(&l), "{r:?} down/up");
            }
        }
    }

    #[test]
    fn moves_never_add_or_drop_tags_or_groups() {
        let l = sample();
        let before = sorted_tag_ids(&l);
        let mut refs: Vec<ItemRef> = before.iter().map(|&i| ItemRef::Tag(i)).collect();
        refs.push(ItemRef::Group(1));
        refs.push(ItemRef::Group(2));
        for r in refs {
            for d in [Up, Down] {
                if let Some(m) = move_item(&l, r, d) {
                    assert_eq!(sorted_tag_ids(&m), before);
                    let groups = m
                        .items
                        .iter()
                        .filter(|i| matches!(i, LayoutItem::Group(_)))
                        .count();
                    assert_eq!(groups, 2);
                }
            }
        }
    }

    #[test]
    fn repeated_down_walks_a_tag_through_every_position() {
        let mut l = layout(vec![t("a"), g(1, &["b"]), t("c")]);
        let mut seen = vec![show(&l)];
        while let Some(next) = move_item(&l, tid("a"), Down) {
            l = next;
            seen.push(show(&l));
        }
        assert_eq!(
            seen,
            [
                "a G1[b] c",
                "G1[a,b] c",
                "G1[b,a] c",
                "G1[b] a c",
                "G1[b] c a"
            ]
        );
    }

    #[test]
    fn remove_group_replaces_it_with_its_tags_in_place() {
        let l = sample();
        assert_eq!(show(&remove_group(&l, 1)), "a b c d G2[] e");
        assert_eq!(show(&remove_group(&l, 2)), "a G1[b,c] d e");
        assert_eq!(remove_group(&l, 99), l);
    }

    #[test]
    fn insert_group_at_top_adds_an_empty_group_first() {
        let l = layout(vec![t("a")]);
        assert_eq!(show(&insert_group_at_top(&l, 7, "x")), "G7[] a");
        assert_eq!(
            show(&insert_group_at_top(&TagLayout::default(), 7, "x")),
            "G7[]"
        );
    }

    #[test]
    fn assignments_number_top_level_and_each_group_from_zero() {
        let a = assignments(&sample());
        let tags: Vec<(i64, Option<i64>, i64)> = a
            .tags
            .iter()
            .map(|x| (x.id, x.group_id, x.sort_order))
            .collect();
        assert_eq!(
            tags,
            vec![
                (tag("a").id, None, 0),
                (tag("b").id, Some(1), 0),
                (tag("c").id, Some(1), 1),
                (tag("d").id, None, 2),
                (tag("e").id, None, 4),
            ]
        );
        let groups: Vec<(i64, i64)> = a.groups.iter().map(|x| (x.id, x.sort_order)).collect();
        assert_eq!(groups, vec![(1, 1), (2, 3)]);
    }

    #[test]
    fn assignments_of_an_empty_layout_are_empty() {
        assert_eq!(assignments(&TagLayout::default()), Assignments::default());
    }

    #[test]
    fn top_level_numbers_are_consecutive_across_tags_and_groups() {
        let a = assignments(&sample());
        let mut top: Vec<i64> = a
            .tags
            .iter()
            .filter(|x| x.group_id.is_none())
            .map(|x| x.sort_order)
            .chain(a.groups.iter().map(|x| x.sort_order))
            .collect();
        top.sort();
        assert_eq!(top, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn flatten_expands_groups_in_place() {
        let l = sample();
        let names: Vec<&str> = flatten(&l).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d", "e"]);
    }

    fn row(
        id: i64,
        name: &str,
        order: Option<i64>,
        group: Option<i64>,
    ) -> (i64, String, Option<i64>, Option<i64>) {
        (id, name.to_string(), order, group)
    }

    fn grow(id: i64, name: &str, order: Option<i64>) -> (i64, String, Option<i64>) {
        (id, name.to_string(), order)
    }

    #[test]
    fn build_layout_without_groups_orders_by_number_then_name() {
        let tags = [
            row(1, "d", None, None),
            row(2, "b", Some(1), None),
            row(3, "a", None, None),
            row(4, "c", Some(0), None),
        ];
        let l = build_layout(&[], &tags);
        assert_eq!(show(&l), "c b a d");
    }

    #[test]
    fn build_layout_shares_one_number_line_between_tags_and_groups() {
        let groups = [grow(1, "g", Some(1))];
        let tags = [
            row(10, "t0", Some(0), None),
            row(11, "t2", Some(2), None),
            row(12, "in2", Some(1), Some(1)),
            row(13, "in1", Some(0), Some(1)),
        ];
        let l = build_layout(&groups, &tags);
        assert_eq!(show(&l), "t0 G1[in1,in2] t2");
    }

    #[test]
    fn build_layout_puts_unplaced_after_placed_by_name_for_groups_too() {
        let groups = [
            grow(1, "zg", None),
            grow(2, "ag", None),
            grow(3, "pg", Some(5)),
        ];
        let tags = [
            row(10, "m", None, None),
            row(11, "x", Some(9), None),
            row(12, "n2", None, Some(1)),
            row(13, "n1", None, Some(1)),
            row(14, "p", Some(3), Some(1)),
        ];
        let l = build_layout(&groups, &tags);
        // placed: pg(5), x(9); then unplaced by name: ag, m, zg
        assert_eq!(show(&l), "G3[] x G2[] m G1[p,n1,n2]");
    }

    #[test]
    fn build_layout_treats_a_dangling_group_id_as_top_level() {
        let groups = [grow(1, "g", Some(0))];
        let tags = [
            row(10, "a", Some(1), Some(99)),
            row(11, "b", Some(2), Some(1)),
        ];
        let l = build_layout(&groups, &tags);
        assert_eq!(show(&l), "G1[b] a");
        assert_eq!(sorted_tag_ids(&l), vec![10, 11]);
    }

    #[test]
    fn build_layout_keeps_empty_groups_and_is_empty_for_no_rows() {
        assert_eq!(show(&build_layout(&[grow(1, "g", None)], &[])), "G1[]");
        assert_eq!(build_layout(&[], &[]), TagLayout::default());
    }

    #[test]
    fn assignments_round_trip_through_build_layout() {
        let l = sample();
        let a = assignments(&l);
        let groups: Vec<_> = l
            .items
            .iter()
            .filter_map(|i| match i {
                LayoutItem::Group(g) => Some(g),
                _ => None,
            })
            .map(|g| {
                let o = a.groups.iter().find(|x| x.id == g.id).unwrap().sort_order;
                (g.id, g.name.clone(), Some(o))
            })
            .collect();
        let tags: Vec<_> = flatten(&l)
            .iter()
            .map(|t| {
                let x = a.tags.iter().find(|x| x.id == t.id).unwrap();
                (t.id, t.name.clone(), Some(x.sort_order), x.group_id)
            })
            .collect();
        assert_eq!(build_layout(&groups, &tags), l);
    }

    #[test]
    fn parse_helpers_accept_only_known_words() {
        assert_eq!(ItemRef::parse("tag", 1), Some(ItemRef::Tag(1)));
        assert_eq!(ItemRef::parse("group", 2), Some(ItemRef::Group(2)));
        assert_eq!(ItemRef::parse("Tag", 1), None);
        assert_eq!(Direction::parse("up"), Some(Up));
        assert_eq!(Direction::parse("down"), Some(Down));
        assert_eq!(Direction::parse("left"), None);
    }
}
