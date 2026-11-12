import {
  useCallback,
  useEffect,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type RefObject,
} from "react";
import {
  clearSelection,
  clickSelect,
  pruneSelection,
  selectAll,
  selectionForContextMenu,
  type ClickModifiers,
  type SelectionState,
} from "./lib/multiSelect";
import { isTextEntryTarget } from "./lib/textEntryTarget";
import type { VideoDto } from "./types";

// 一覧の複数選択(クリック・Ctrl/⌘/Shift のクリック・Ctrl/⌘+A・Esc、取り直しで
// 消えた行の整理、件数の通知)。listRef はキー操作を受ける一覧のスクロール要素で、
// 修飾キー付きのクリックでフォーカスを移す。
export function useListSelection(
  videos: VideoDto[],
  onSelectionCountChange: (count: number) => void,
  listRef: RefObject<HTMLDivElement | null>,
) {
  // Selection only highlights rows (via the CSS classes VideoRow applies
  // itself). Held by id so rows scrolled out of the virtualizer's window stay
  // selected.
  const [selection, setSelection] = useState<SelectionState>(clearSelection);

  useEffect(() => {
    onSelectionCountChange(selection.ids.size);
  }, [selection.ids.size, onSelectionCountChange]);

  // 取り直した一覧に合わせて選択を整理する。setSelection しか使わないので
  // 描画をまたいで同じ関数のまま。
  // Only videos that vanished from the list (e.g. a watch folder was
  // removed) are dropped; a plain refresh must not wipe the user's
  // selection.
  const prune = useCallback((rows: VideoDto[]) => {
    setSelection((prev) =>
      pruneSelection(
        prev,
        rows.map((v) => v.id),
      ),
    );
  }, []);

  // 選択を全部外す。setSelection しか使わないので描画をまたいで同じ関数のまま。
  const clear = useCallback(() => {
    setSelection(clearSelection());
  }, []);

  function handleSelect(video: VideoDto, mods: ClickModifiers) {
    const orderedIds = videos.map((v) => v.id);
    setSelection((prev) => clickSelect(prev, orderedIds, video.id, mods));
  }

  // 右クリックした行に合わせて選択を決め、メニューの対象にする id を返す。
  function selectForContextMenu(videoId: string): string[] {
    const next = selectionForContextMenu(selection, videoId);
    setSelection(next);
    return [...next.ids];
  }

  function handleKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (isTextEntryTarget(e.target)) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      setSelection(selectAll(videos.map((v) => v.id)));
    } else if (e.key === "Escape") {
      setSelection(clearSelection());
    }
  }

  // Shift/Ctrl+click would otherwise start the browser's own text selection.
  // preventDefault also suppresses the focus change, and the keyboard
  // shortcuts need the list focused, so focus it by hand.
  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (!(e.ctrlKey || e.metaKey || e.shiftKey)) return;
    if (e.target instanceof Element && e.target.closest("input, textarea, select, button, a")) {
      return;
    }
    e.preventDefault();
    listRef.current?.focus({ preventScroll: true });
  }

  return {
    selectedIds: selection.ids,
    multiSelected: selection.ids.size >= 2,
    prune,
    clear,
    handleSelect,
    selectForContextMenu,
    handleKeyDown,
    handleMouseDown,
  };
}
