// computeVisibleChipCount のユニットテスト。src/lib/sidebarResize.test.ts と
// 同じパターン(Node組み込みtest runnerを
// `node --experimental-strip-types --test`で実行、package.jsonのtest:unit
// 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { computeVisibleChipCount } from "./tagBarLayout.ts";

test("全チップの合計幅がavailableに収まる場合は▾ボタン分を差し引かず全件返す", () => {
  // available = 1000 - 32(APP_HORIZONTAL_PADDING_PX) - 50(clearAll) = 918
  // 合計幅 300 <= 918 なので、overflowButtonWidthPx(30)を差し引かず全件(3件)返る。
  assert.equal(computeVisibleChipCount(1000, 50, 30, [100, 100, 100]), 3);
});

test("▾ボタン分の予約は「収まらない場合」だけに発生する(収まる場合は予約しない)", () => {
  // available = 132 - 32 - 0 = 100。合計幅100 <= available(100)なので全件返る。
  // もしoverflowButtonWidthPx(30)を誤って差し引いていれば0件になってしまうケース。
  assert.equal(computeVisibleChipCount(132, 0, 30, [100]), 1);
});

test("境界値: budgetにちょうど収まる幅では2件、1px超える幅では1件になる", () => {
  // available = 112 - 32 - 0 = 80、budget = 80 - 10(overflowButtonWidthPx) = 70。
  // 合計幅90 > available(80)なので▾ボタン分を差し引いた70が予算になる。
  // chipWidthsPx=[40,30,20]の累積は40,70,90 -- budget=70ちょうどで2件目まで収まる。
  assert.equal(computeVisibleChipCount(112, 0, 10, [40, 30, 20]), 2);

  // windowInnerWidthを1px減らす(111)とavailable=79、budget=69となり、
  // 累積70(1+2番目)は69を1pxだけ超えるため1件しか収まらなくなる。
  assert.equal(computeVisibleChipCount(111, 0, 10, [40, 30, 20]), 1);
});

test("空配列を渡すと0を返す", () => {
  assert.equal(computeVisibleChipCount(1280, 50, 30, []), 0);
});

test("最初の1個すら収まらない極端に狭いウィンドウ幅では0を返す", () => {
  // available = 0 - 32 - 0 = -32 (負数)。budget = -32 - 0 = -32。
  // 先頭のチップ(50px)すら収まらない。
  assert.equal(computeVisibleChipCount(0, 0, 0, [50]), 0);
});

test("clearAllWidthPx単体でavailableを超える極端なケースでは0を返す", () => {
  // available = 100 - 32 - 200(clearAllWidthPx) = -132 (負数)。
  // チップが1つもバーに収まらない。
  assert.equal(computeVisibleChipCount(100, 200, 0, [10]), 0);
});
