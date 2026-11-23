// createTrailingDebounce / createThrottledSink のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { createThrottledSink, createTrailingDebounce, type Clock } from "./timing.ts";

// 時刻を手で進められる偽の時計。
function fakeClock() {
  let time = 1000;
  let nextId = 1;
  const timers = new Map<number, { at: number; fn: () => void }>();
  const clock: Clock = {
    now: () => time,
    setTimeout(fn, ms) {
      const id = nextId++;
      timers.set(id, { at: time + ms, fn });
      return id;
    },
    clearTimeout(handle) {
      timers.delete(handle as number);
    },
  };
  const advance = (ms: number) => {
    const target = time + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, t]) => t.at <= target)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      time = due[1].at;
      due[1].fn();
    }
    time = target;
  };
  return { clock, advance, pendingTimers: () => timers.size };
}

test("デバウンス: 連続した呼び出しは、最後から待った後の1回にまとまる", () => {
  const { clock, advance } = fakeClock();
  let calls = 0;
  const d = createTrailingDebounce(() => (calls += 1), 500, clock);
  d.call();
  advance(300);
  d.call();
  advance(499);
  assert.equal(calls, 0);
  advance(1);
  assert.equal(calls, 1);
  advance(5000);
  assert.equal(calls, 1);
});

test("デバウンス: cancel すると呼ばれない", () => {
  const { clock, advance, pendingTimers } = fakeClock();
  let calls = 0;
  const d = createTrailingDebounce(() => (calls += 1), 500, clock);
  d.call();
  d.cancel();
  advance(1000);
  assert.equal(calls, 0);
  assert.equal(pendingTimers(), 0);
});

const isDone = (v: number) => v === 100;

test("間引き: 最初の値はすぐ反映される", () => {
  const { clock } = fakeClock();
  const seen: number[] = [];
  const s = createThrottledSink((v: number) => seen.push(v), 250, isDone, clock);
  s.push(1);
  assert.deepEqual(seen, [1]);
});

test("間引き: 間隔内の値は最新だけが、間隔が空いた時点で反映される", () => {
  const { clock, advance } = fakeClock();
  const seen: number[] = [];
  const s = createThrottledSink((v: number) => seen.push(v), 250, isDone, clock);
  s.push(1);
  advance(50);
  s.push(2);
  advance(50);
  s.push(3);
  assert.deepEqual(seen, [1]);
  advance(149);
  assert.deepEqual(seen, [1]);
  advance(1);
  assert.deepEqual(seen, [1, 3], "中間値 2 は捨てられる");
});

test("間引き: 間隔が空いていれば次の値もすぐ反映される", () => {
  const { clock, advance } = fakeClock();
  const seen: number[] = [];
  const s = createThrottledSink((v: number) => seen.push(v), 250, isDone, clock);
  s.push(1);
  advance(250);
  s.push(2);
  assert.deepEqual(seen, [1, 2]);
});

test("間引き: 即反映の値(最終値)は間隔内でも反映され、保留中の古い値は後から出ない", () => {
  const { clock, advance, pendingTimers } = fakeClock();
  const seen: number[] = [];
  const s = createThrottledSink((v: number) => seen.push(v), 250, isDone, clock);
  s.push(1);
  s.push(2);
  s.push(100);
  assert.deepEqual(seen, [1, 100]);
  assert.equal(pendingTimers(), 0);
  advance(1000);
  assert.deepEqual(seen, [1, 100], "最終値のあとに古い 2 が上書きしない");
});

test("間引き: cancel すると保留中の値は反映されない", () => {
  const { clock, advance } = fakeClock();
  const seen: number[] = [];
  const s = createThrottledSink((v: number) => seen.push(v), 250, isDone, clock);
  s.push(1);
  s.push(2);
  s.cancel();
  advance(1000);
  assert.deepEqual(seen, [1]);
});
