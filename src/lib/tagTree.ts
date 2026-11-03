// タグの親子(2階層まで)の純粋ロジック。表示順の規則は gb-core の
// tag_hierarchy::display_order と同じ。バックエンドは sort_order → 名前の順で
// 平らな一覧を返すので、ここでは渡された順を兄弟の中でそのまま保つ
// (sort_order の値自体はフロントに来ない)。

export interface TagLike {
  id: number;
  parent_id: number | null;
}

export interface TagTreeNode<T extends TagLike> {
  tag: T;
  children: T[];
}

export interface TagLayoutItem {
  id: number;
  parent_id: number | null;
}

/**
 * 表示上の親の id。親が存在しない・自分自身・親自身がまた子(存在する親を持つ)の
 * ときは null(トップレベル扱い)。古いデータでもタグが一覧から消えないための規則。
 */
function effectiveParents<T extends TagLike>(tags: T[]): Map<number, number | null> {
  const byId = new Map(tags.map((t) => [t.id, t]));
  const result = new Map<number, number | null>();
  for (const t of tags) {
    let parent: number | null = null;
    if (t.parent_id !== null && t.parent_id !== t.id) {
      const p = byId.get(t.parent_id);
      if (p && (p.parent_id === null || !byId.has(p.parent_id))) parent = p.id;
    }
    result.set(t.id, parent);
  }
  return result;
}

/** 親(トップレベル)の順に並んだ木。兄弟の順は入力の順のまま。 */
export function buildTagTree<T extends TagLike>(tags: T[]): TagTreeNode<T>[] {
  const parents = effectiveParents(tags);
  const nodes = new Map<number, TagTreeNode<T>>();
  const top: TagTreeNode<T>[] = [];
  for (const tag of tags) {
    if (parents.get(tag.id) === null) {
      const node = { tag, children: [] };
      nodes.set(tag.id, node);
      top.push(node);
    }
  }
  for (const tag of tags) {
    const parent = parents.get(tag.id);
    if (parent !== null && parent !== undefined) nodes.get(parent)?.children.push(tag);
  }
  return top;
}

function flatten<T extends TagLike>(tree: TagTreeNode<T>[]): T[] {
  return tree.flatMap((node) => [
    node.tag,
    ...node.children.map((c) => ({ ...c, parent_id: node.tag.id })),
  ]);
}

/** 表示順(親の直後にその子)の平らな配列。parent_id は表示上の親に正規化する。 */
export function displayOrder<T extends TagLike>(tags: T[]): T[] {
  return flatten(
    buildTagTree(tags).map((node) => ({
      tag: node.tag.parent_id === null ? node.tag : { ...node.tag, parent_id: null },
      children: node.children,
    })),
  );
}

/** 子を持つタグか(表示上の親か)。 */
export function hasChildren<T extends TagLike>(tags: T[], tagId: number): boolean {
  return buildTagTree(tags).some((n) => n.tag.id === tagId && n.children.length > 0);
}

/**
 * 親の候補: トップレベルで、自分でなく、自分が子を持たないとき。
 * 子を持つタグは別のタグの子にできないので、空になる。
 */
export function parentCandidates<T extends TagLike>(tags: T[], tagId: number): T[] {
  if (hasChildren(tags, tagId)) return [];
  return buildTagTree(tags)
    .map((n) => n.tag)
    .filter((t) => t.id !== tagId);
}

/**
 * タグの親を変える。候補以外は何もしない(同じ配列を返す)。
 * 親の下に移すときは最後の子、なしにするときはトップレベルの最後に置く。
 */
export function setParent<T extends TagLike>(
  tags: T[],
  tagId: number,
  parentId: number | null,
): T[] {
  const ordered = displayOrder(tags);
  const target = ordered.find((t) => t.id === tagId);
  if (!target || target.parent_id === parentId) return tags;
  if (parentId !== null && !parentCandidates(ordered, tagId).some((t) => t.id === parentId)) {
    return tags;
  }
  const rest = ordered.filter((t) => t.id !== tagId);
  return displayOrder([...rest, { ...target, parent_id: parentId }]);
}

type Direction = "up" | "down";

function siblingIndex<T extends TagLike>(
  tags: T[],
  tagId: number,
): { siblings: number[]; at: number; parent: number | null } | null {
  const ordered = displayOrder(tags);
  const target = ordered.find((t) => t.id === tagId);
  if (!target) return null;
  const siblings = ordered.filter((t) => t.parent_id === target.parent_id).map((t) => t.id);
  return { siblings, at: siblings.indexOf(tagId), parent: target.parent_id };
}

/** 兄弟の中で、その方向に動けるか。 */
export function canMove<T extends TagLike>(tags: T[], tagId: number, direction: Direction): boolean {
  const info = siblingIndex(tags, tagId);
  if (!info) return false;
  return direction === "up" ? info.at > 0 : info.at < info.siblings.length - 1;
}

/** 兄弟の中で1つ動かす。親は子ごと動く。動けないときは同じ配列を返す。 */
export function moveWithinSiblings<T extends TagLike>(
  tags: T[],
  tagId: number,
  direction: Direction,
): T[] {
  if (!canMove(tags, tagId, direction)) return tags;
  const info = siblingIndex(tags, tagId);
  if (!info) return tags;
  const swapWith = info.siblings[direction === "up" ? info.at - 1 : info.at + 1];
  const tree = buildTagTree(displayOrder(tags));
  if (info.parent === null) {
    const a = tree.findIndex((n) => n.tag.id === tagId);
    const b = tree.findIndex((n) => n.tag.id === swapWith);
    [tree[a], tree[b]] = [tree[b], tree[a]];
  } else {
    const node = tree.find((n) => n.tag.id === info.parent);
    if (!node) return tags;
    const a = node.children.findIndex((c) => c.id === tagId);
    const b = node.children.findIndex((c) => c.id === swapWith);
    [node.children[a], node.children[b]] = [node.children[b], node.children[a]];
  }
  return flatten(tree);
}

/** タグを消した後の下書き。子は親なしに戻る(バックエンドの削除と同じ)。 */
export function removeTag<T extends TagLike>(tags: T[], tagId: number): T[] {
  return displayOrder(
    tags
      .filter((t) => t.id !== tagId)
      .map((t) => (t.parent_id === tagId ? { ...t, parent_id: null } : t)),
  );
}

/** 保存用: 表示順(親の直後にその子)の { id, parent_id } の配列。 */
export function toLayoutItems<T extends TagLike>(tags: T[]): TagLayoutItem[] {
  return displayOrder(tags).map((t) => ({ id: t.id, parent_id: t.parent_id }));
}
