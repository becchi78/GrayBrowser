import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { buildTagDeleteMessage } from "./tagDeleteMessage.ts";

describe("buildTagDeleteMessage", () => {
  it("includes the video count when the tag is in use", () => {
    assert.equal(
      buildTagDeleteMessage("風景", 3),
      "タグ『風景』を削除します。このタグが付いている 3 件の動画からタグが外れます。元に戻せません。",
    );
  });

  it("includes the count sentence for exactly one video", () => {
    assert.match(buildTagDeleteMessage("a", 1), /1 件の動画/);
  });

  it("omits the count sentence when no video has the tag", () => {
    assert.equal(buildTagDeleteMessage("風景", 0), "タグ『風景』を削除します。元に戻せません。");
  });
});
