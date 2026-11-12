import { useEffect, useLayoutEffect, useRef } from "react";
import { jumpTargetIndex } from "./lib/jumpToPercent";

type ScrollToIndex = (index: number, options: { align: "start" | "end" }) => void;

// 一覧の何%の位置へ動かす要求を受けて、その行へスクロールする。100% は最後の
// 行を下端に、それ以外は対象の行を上端に合わせる。
export function useJumpToPercent(
  jumpRequest: { percent: number; seq: number } | null,
  count: number,
  scrollToIndex: ScrollToIndex,
) {
  // Keyed on the request object only: depending on the row count would yank
  // the user back to the jump target whenever the list refetches, so the
  // latest count (and scrollToIndex) is read through a ref instead. The refs
  // are updated in a layout effect, which runs before the jump effect below.
  const countRef = useRef(count);
  const scrollToIndexRef = useRef(scrollToIndex);
  useLayoutEffect(() => {
    countRef.current = count;
    scrollToIndexRef.current = scrollToIndex;
  });

  useEffect(() => {
    if (jumpRequest === null) return;
    const index = jumpTargetIndex(countRef.current, jumpRequest.percent);
    if (index === null) return;
    scrollToIndexRef.current(index, {
      align: jumpRequest.percent === 100 ? "end" : "start",
    });
  }, [jumpRequest]);
}
