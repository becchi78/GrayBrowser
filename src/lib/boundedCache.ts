// thumbnailCache（ThumbnailGrid.tsx）の無制限増加を防ぐための、シンプルな
// FIFO的エビクション。
//
// 実測では正確なメモリ増分は測定ノイズにより確定できなかったが、エビクション
// が一切存在しないこと自体がコード上明確な設計の穴だったため、低コストな
// 上限化のみを実施する（本格的なLRUライブラリの導入は過剰実装として見送り）。
//
// JavaScriptのMapは挿入順序を保持するため、上限を超えたら「最も古く挿入された
// エントリ」を Map.keys().next().value で取り出して削除するだけで、
// アクセス頻度を追跡する本格的なLRUを実装せずに素朴なキャッシュ上限を実現できる。

// 一度に画面へ表示される件数（列数×可視行数＋overscan）は通常数十件程度。
// その数十倍の余裕を持たせておけば、一般的なスクロール操作で
// 直前まで表示していたサムネイルがキャッシュから追い出されて
// 再取得（IPC往復）が頻発することはない、という想定の値。
export const THUMBNAIL_CACHE_MAX_ENTRIES = 2000;

/**
 * cache に value をセットしたうえで、サイズが maxEntries を超えていたら
 * 最も古く追加されたエントリから順に削除して maxEntries 以下に戻す。
 */
export function setWithEviction<K, V>(cache: Map<K, V>, key: K, value: V, maxEntries: number): void {
  cache.set(key, value);
  while (cache.size > maxEntries) {
    const oldestKey = cache.keys().next().value;
    if (oldestKey === undefined) {
      break;
    }
    cache.delete(oldestKey);
  }
}
