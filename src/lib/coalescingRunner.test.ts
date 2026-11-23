// createCoalescingRunner のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { createCoalescingRunner } from "./coalescingRunner.ts";

// 呼ばれるたびに、手動で完了させられる処理を作る。
function manualRun() {
  const resolvers: Array<() => void> = [];
  const run = () =>
    new Promise<void>((resolve) => {
      resolvers.push(resolve);
    });
  return { run, resolvers };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

test("実行中でなければ、要求ですぐ実行する", async () => {
  const { run, resolvers } = manualRun();
  const runner = createCoalescingRunner(run);
  runner.request();
  await flush();
  assert.equal(resolvers.length, 1);
});

test("実行中の連続した要求は、完了後の1回の再実行にまとまる", async () => {
  const { run, resolvers } = manualRun();
  const runner = createCoalescingRunner(run);
  runner.request();
  await flush();
  runner.request();
  runner.request();
  runner.request();
  await flush();
  assert.equal(resolvers.length, 1, "実行中は重ねて走らせない");
  resolvers[0]();
  await flush();
  assert.equal(resolvers.length, 2, "完了後に1回だけ再実行する");
  resolvers[1]();
  await flush();
  assert.equal(resolvers.length, 2, "予約が尽きたら止まる");
});

test("予約が無ければ、完了しても再実行しない", async () => {
  const { run, resolvers } = manualRun();
  const runner = createCoalescingRunner(run);
  runner.request();
  await flush();
  resolvers[0]();
  await flush();
  assert.equal(resolvers.length, 1);
});

test("完了後の要求は、またすぐ実行される", async () => {
  const { run, resolvers } = manualRun();
  const runner = createCoalescingRunner(run);
  runner.request();
  await flush();
  resolvers[0]();
  await flush();
  runner.request();
  await flush();
  assert.equal(resolvers.length, 2);
});

test("cancel すると、実行中の予約は実行されず、以後の要求も無視する", async () => {
  const { run, resolvers } = manualRun();
  const runner = createCoalescingRunner(run);
  runner.request();
  await flush();
  runner.request();
  runner.cancel();
  resolvers[0]();
  await flush();
  assert.equal(resolvers.length, 1);
  runner.request();
  await flush();
  assert.equal(resolvers.length, 1);
});

test("実行が reject しても調停は止まらず、予約は実行される", async () => {
  let calls = 0;
  const rejects: Array<() => void> = [];
  const runner = createCoalescingRunner(() => {
    calls += 1;
    return new Promise<void>((_, reject) => {
      rejects.push(() => reject(new Error("boom")));
    });
  });
  runner.request();
  await flush();
  runner.request();
  rejects[0]();
  await flush();
  assert.equal(calls, 2);
});

test("実行が同期的に例外を投げても、次の要求で実行できる", async () => {
  let calls = 0;
  const runner = createCoalescingRunner(() => {
    calls += 1;
    throw new Error("sync");
  });
  runner.request();
  await flush();
  runner.request();
  await flush();
  assert.equal(calls, 2);
});
