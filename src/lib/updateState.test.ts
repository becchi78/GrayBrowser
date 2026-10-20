import assert from "node:assert/strict";
import test from "node:test";
import {
  canDismiss,
  errorReason,
  formatBytes,
  INITIAL_UPDATE_STATE,
  isBusy,
  progressPercent,
  updateReducer,
} from "./updateState.ts";
import type { AvailableUpdate, UpdateAction, UpdateState } from "./updateState.ts";

const UPDATE: AvailableUpdate = { version: "0.9.0", currentVersion: "0.8.5", notes: "## 追加\n- a" };

function run(actions: UpdateAction[], from: UpdateState = INITIAL_UPDATE_STATE): UpdateState {
  return actions.reduce(updateReducer, from);
}

test("check with no update ends up to date with the current version", () => {
  const s = run([{ type: "checkSucceeded", result: null, currentVersion: "0.8.5" }]);
  assert.deepEqual(s, { kind: "upToDate", currentVersion: "0.8.5" });
});

test("check with an update shows it as available", () => {
  const s = run([{ type: "checkSucceeded", result: UPDATE, currentVersion: "0.8.5" }]);
  assert.deepEqual(s, { kind: "available", update: UPDATE });
});

test("check failure keeps the reason and carries no update", () => {
  const s = run([{ type: "checkFailed", reason: "offline" }]);
  assert.deepEqual(s, { kind: "error", phase: "check", reason: "offline", update: null });
});

test("a late check result is ignored once the state has moved on", () => {
  const available = run([{ type: "checkSucceeded", result: UPDATE, currentVersion: "0.8.5" }]);
  assert.equal(updateReducer(available, { type: "checkFailed", reason: "x" }), available);
  assert.equal(updateReducer(available, { type: "checkSucceeded", result: null, currentVersion: "1" }), available);
});

test("download starts only from available or a failed download", () => {
  assert.equal(updateReducer(INITIAL_UPDATE_STATE, { type: "downloadStarted" }), INITIAL_UPDATE_STATE);
  const upToDate: UpdateState = { kind: "upToDate", currentVersion: "1" };
  assert.equal(updateReducer(upToDate, { type: "downloadStarted" }), upToDate);
  const checkError: UpdateState = { kind: "error", phase: "check", reason: "r", update: null };
  assert.equal(updateReducer(checkError, { type: "downloadStarted" }), checkError);
  const started = updateReducer({ kind: "available", update: UPDATE }, { type: "downloadStarted" });
  assert.deepEqual(started, { kind: "downloading", update: UPDATE, downloaded: 0, total: null });
  const retried = updateReducer(
    { kind: "error", phase: "download", reason: "r", update: UPDATE },
    { type: "downloadStarted" },
  );
  assert.equal(retried.kind, "downloading");
});

test("a second downloadStarted while downloading is ignored (no double operation)", () => {
  const downloading = run([{ type: "downloadStarted" }], { kind: "available", update: UPDATE });
  assert.equal(updateReducer(downloading, { type: "downloadStarted" }), downloading);
});

test("rechecking is refused while downloading or installing", () => {
  const downloading = run([{ type: "downloadStarted" }], { kind: "available", update: UPDATE });
  assert.equal(updateReducer(downloading, { type: "checkStarted" }), downloading);
  const installing = updateReducer(downloading, { type: "downloadEvent", event: { event: "Finished" } });
  assert.equal(updateReducer(installing, { type: "checkStarted" }), installing);
  assert.deepEqual(updateReducer({ kind: "upToDate", currentVersion: "1" }, { type: "checkStarted" }), {
    kind: "checking",
  });
});

test("download events accumulate bytes and finish into installing", () => {
  const s = run(
    [
      { type: "downloadStarted" },
      { type: "downloadEvent", event: { event: "Started", data: { contentLength: 1000 } } },
      { type: "downloadEvent", event: { event: "Progress", data: { chunkLength: 300 } } },
      { type: "downloadEvent", event: { event: "Progress", data: { chunkLength: 200 } } },
    ],
    { kind: "available", update: UPDATE },
  );
  assert.deepEqual(s, { kind: "downloading", update: UPDATE, downloaded: 500, total: 1000 });
  assert.deepEqual(updateReducer(s, { type: "downloadEvent", event: { event: "Finished" } }), {
    kind: "installing",
    update: UPDATE,
  });
});

test("unknown or zero content length keeps total null; negative chunks do not shrink progress", () => {
  const base = run([{ type: "downloadStarted" }], { kind: "available", update: UPDATE });
  for (const data of [{}, { contentLength: 0 }]) {
    const s = updateReducer(base, { type: "downloadEvent", event: { event: "Started", data } });
    assert.equal(s.kind === "downloading" && s.total, null);
  }
  const s = updateReducer(base, { type: "downloadEvent", event: { event: "Progress", data: { chunkLength: -5 } } });
  assert.equal(s.kind === "downloading" && s.downloaded, 0);
});

test("download events outside downloading are ignored", () => {
  const s: UpdateState = { kind: "available", update: UPDATE };
  assert.equal(updateReducer(s, { type: "downloadEvent", event: { event: "Progress", data: { chunkLength: 1 } } }), s);
});

test("download failure keeps the update so it can be retried; ignored outside a download", () => {
  const downloading = run([{ type: "downloadStarted" }], { kind: "available", update: UPDATE });
  assert.deepEqual(updateReducer(downloading, { type: "downloadFailed", reason: "bad sig" }), {
    kind: "error",
    phase: "download",
    reason: "bad sig",
    update: UPDATE,
  });
  const installing: UpdateState = { kind: "installing", update: UPDATE };
  assert.equal(updateReducer(installing, { type: "downloadFailed", reason: "x" }).kind, "error");
  const checking = INITIAL_UPDATE_STATE;
  assert.equal(updateReducer(checking, { type: "downloadFailed", reason: "x" }), checking);
});

test("progressPercent: unknown total, bounds, and overshoot", () => {
  assert.equal(progressPercent(10, null), null);
  assert.equal(progressPercent(10, 0), null);
  assert.equal(progressPercent(10, -1), null);
  assert.equal(progressPercent(0, 100), 0);
  assert.equal(progressPercent(50, 100), 50);
  assert.equal(progressPercent(999, 1000), 99);
  assert.equal(progressPercent(1000, 1000), 100);
  assert.equal(progressPercent(5000, 1000), 100);
  assert.equal(progressPercent(-5, 1000), 0);
  assert.equal(progressPercent(1, Number.NaN), null);
});

test("busy and dismiss rules", () => {
  const busy: UpdateState[] = [
    { kind: "downloading", update: UPDATE, downloaded: 0, total: null },
    { kind: "installing", update: UPDATE },
  ];
  const idle: UpdateState[] = [
    INITIAL_UPDATE_STATE,
    { kind: "upToDate", currentVersion: "1" },
    { kind: "available", update: UPDATE },
    { kind: "error", phase: "check", reason: "r", update: null },
  ];
  for (const s of busy) {
    assert.equal(isBusy(s), true);
    assert.equal(canDismiss(s), false);
  }
  for (const s of idle) {
    assert.equal(isBusy(s), false);
    assert.equal(canDismiss(s), true);
  }
});

test("formatBytes", () => {
  assert.equal(formatBytes(0), "0 KB");
  assert.equal(formatBytes(1536), "2 KB");
  assert.equal(formatBytes(5 * 1024 * 1024), "5.0 MB");
  assert.equal(formatBytes(-1), "0 KB");
});

test("errorReason handles strings, Errors and other values", () => {
  assert.equal(errorReason("boom"), "boom");
  assert.equal(errorReason(new Error("bad")), "bad");
  assert.equal(errorReason(42), "42");
});
