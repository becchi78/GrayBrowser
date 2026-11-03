import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { buildTagDeleteMessage } from "./tagDeleteMessage.ts";

describe("buildTagDeleteMessage", () => {
  it("includes the video count when the tag is in use", () => {
    assert.equal(
      buildTagDeleteMessage(3),
      "このタグが付いている 3 件の動画からタグが外れます。元に戻せません。",
    );
  });

  it("includes the count sentence for exactly one video", () => {
    assert.match(buildTagDeleteMessage(1), /1 件の動画/);
  });

  it("omits the count sentence when no video has the tag", () => {
    assert.equal(buildTagDeleteMessage(0), "元に戻せません。");
  });

  it("mentions child tags that return to the top level", () => {
    assert.equal(
      buildTagDeleteMessage(2, 3),
      "このタグが付いている 2 件の動画からタグが外れます。子のタグ 3 件は削除されず、親なしに戻ります。元に戻せません。",
    );
    assert.equal(
      buildTagDeleteMessage(0, 1),
      "子のタグ 1 件は削除されず、親なしに戻ります。元に戻せません。",
    );
  });
});
