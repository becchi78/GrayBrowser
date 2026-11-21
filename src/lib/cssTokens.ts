// CSS のカスタムプロパティ(長さ)を px の数値として読む。getPropertyValue は
// calc() や var() を解決する前の文字列を返すため、実際に長さとして使う要素を
// 作り、計算後の幅を読む。行の高さなどレイアウトの寸法の情報源を CSS に
// 一本化しつつ、JS(仮想スクロールの見積もり等)からも同じ値を使うためのもの。

/**
 * `name`(例: `--row-height`)を px で返す。DOM が無い・変数が未定義・長さに
 * 解決できない場合は `fallbackPx`。
 */
export function readLengthToken(name: string, fallbackPx: number): number {
  if (typeof document === "undefined" || document.body === null) return fallbackPx;
  const probe = document.createElement("div");
  probe.style.cssText = `position:absolute;visibility:hidden;pointer-events:none;width:var(${name})`;
  document.body.appendChild(probe);
  const px = Number.parseFloat(getComputedStyle(probe).width);
  probe.remove();
  // 未定義の変数は width が auto になり、空の絶対配置の要素では 0 になる。
  return Number.isFinite(px) && px > 0 ? px : fallbackPx;
}
