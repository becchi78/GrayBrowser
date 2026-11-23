/**
 * 通知の連打をまとめるための、時間に関する小さな部品。
 * タイマーと時計は差し替えられるようにしてあり、テストでは偽物を渡す。
 */
export interface Clock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const realClock: Clock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface TrailingDebounce {
  call(): void;
  cancel(): void;
}

/** 最後の call() から delayMs 経って初めて fn を1回呼ぶ(後端のデバウンス)。 */
export function createTrailingDebounce(
  fn: () => void,
  delayMs: number,
  clock: Clock = realClock,
): TrailingDebounce {
  let timer: unknown = null;
  return {
    call() {
      if (timer !== null) clock.clearTimeout(timer);
      timer = clock.setTimeout(() => {
        timer = null;
        fn();
      }, delayMs);
    },
    cancel() {
      if (timer !== null) clock.clearTimeout(timer);
      timer = null;
    },
  };
}

export interface ThrottledSink<T> {
  push(value: T): void;
  cancel(): void;
}

/**
 * 値の流れを intervalMs に1回程度に間引いて apply に渡す。
 * - 前回の反映から intervalMs 以上経っていれば、すぐ反映する。
 * - それ以外は最新の値だけを保留し、間隔が空いたところで反映する(中間値は捨てる)。
 * - isImmediate(value) が true の値(完了など)は、間引かずに即反映し、保留中の
 *   古い値は捨てる。最終値が間引きで失われないようにするため。
 */
export function createThrottledSink<T>(
  apply: (value: T) => void,
  intervalMs: number,
  isImmediate: (value: T) => boolean,
  clock: Clock = realClock,
): ThrottledSink<T> {
  let lastApplied = -Infinity;
  let timer: unknown = null;
  let hasPending = false;
  let pending: T | undefined;

  const clearPending = () => {
    if (timer !== null) clock.clearTimeout(timer);
    timer = null;
    hasPending = false;
    pending = undefined;
  };

  const applyNow = (value: T) => {
    lastApplied = clock.now();
    apply(value);
  };

  return {
    push(value) {
      const elapsed = clock.now() - lastApplied;
      if (isImmediate(value) || elapsed >= intervalMs) {
        clearPending();
        applyNow(value);
        return;
      }
      pending = value;
      hasPending = true;
      if (timer === null) {
        timer = clock.setTimeout(() => {
          timer = null;
          if (!hasPending) return;
          const latest = pending as T;
          hasPending = false;
          pending = undefined;
          applyNow(latest);
        }, intervalMs - elapsed);
      }
    },
    cancel: clearPending,
  };
}
