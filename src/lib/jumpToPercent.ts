export const JUMP_PERCENTS: readonly number[] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

// 件数が割り切れないときは切り捨てる(100% だけが末尾に届く)。範囲外の割合は
// 先頭/末尾に丸め、呼び出し側が不正な行番号をvirtualizerへ渡さないようにする。
export function jumpTargetIndex(count: number, percent: number): number | null {
  if (count <= 0) return null;
  const index = Math.floor((count * percent) / 100);
  return Math.min(Math.max(index, 0), count - 1);
}
