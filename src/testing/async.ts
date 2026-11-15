import { act } from "@testing-library/react";

/** 外から解決・失敗させられる Promise。処理中の状態を作るために使う。 */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** 解決しない Promise。処理が終わらない状態を保つために使う。 */
export function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

/** 連なった Promise の解決と、それに続く再描画を落ち着かせる。 */
export async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {});
  }
}
