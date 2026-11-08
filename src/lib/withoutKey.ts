// いずれも元の値を変更せず、新しい値を返す(React の state 更新で参照が
// 変わることに依存するため)。キーが無ければ中身の同じコピーを返す。

export function withoutKey<V>(record: Record<string, V>, key: string): Record<string, V> {
  const next = { ...record };
  delete next[key];
  return next;
}

export function withoutItem<T>(set: ReadonlySet<T>, item: T): Set<T> {
  const next = new Set(set);
  next.delete(item);
  return next;
}
