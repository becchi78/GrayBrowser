import { useMemo } from "react";
import type { MenuPosition } from "../lib/contextMenu";
import { editMenuState } from "../lib/editMenu";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";

export type EditTarget = HTMLInputElement | HTMLTextAreaElement | HTMLElement;

interface Props {
  target: EditTarget;
  position: MenuPosition;
  onClose: () => void;
}

function isFormField(el: EditTarget): el is HTMLInputElement | HTMLTextAreaElement {
  return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
}

// contenteditable has no value/selectionStart; the document selection is the
// only source, and only counts when it lies inside the target.
function readInput(el: EditTarget) {
  if (isFormField(el)) {
    return {
      value: el.value,
      // number/email inputs report null offsets (no selection API), so cut/copy
      // stay disabled there.
      selectionStart: el.selectionStart ?? 0,
      selectionEnd: el.selectionEnd ?? 0,
      readOnly: el.readOnly,
      disabled: el.disabled,
      isPassword: el instanceof HTMLInputElement && el.type === "password",
    };
  }
  const sel = window.getSelection();
  const inside = sel !== null && sel.rangeCount > 0 && el.contains(sel.anchorNode);
  const text = el.textContent ?? "";
  return {
    value: text,
    selectionStart: 0,
    selectionEnd: inside && !sel.isCollapsed ? sel.toString().length : 0,
    readOnly: false,
    disabled: false,
    isPassword: false,
  };
}

function selectAll(el: EditTarget) {
  if (isFormField(el)) {
    el.select();
    return;
  }
  const range = document.createRange();
  range.selectNodeContents(el);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

async function paste(el: EditTarget) {
  const text = await navigator.clipboard.readText();
  // The menu click may have moved focus, and execCommand acts on the focused
  // element; refocus again after the async read. If the target went away (or
  // refused focus) during the await, inserting would land in some other field.
  if (!el.isConnected) return;
  el.focus();
  if (document.activeElement !== el) return;
  // insertText (rather than assigning .value) fires a real input event so
  // React's onChange runs, and keeps the edit on the undo stack.
  document.execCommand("insertText", false, text);
}

function run(label: string, action: () => void | Promise<void>) {
  try {
    const result = action();
    if (result) result.catch((e) => console.error(`${label} failed`, e));
  } catch (e) {
    console.error(`${label} failed`, e);
  }
}

// 入力欄専用の右クリックメニュー。WebView 標準のメニューには「パスワード」
// 「絵文字」など不要な項目が混ざるため、4項目だけの独自メニューに置き換える。
export function EditContextMenu({ target, position, onClose }: Props) {
  // 開いた時点の選択状態で項目の有効/無効を決める(メニュー表示中は入力が
  // 変わらない)。
  const state = useMemo(() => editMenuState(readInput(target)), [target]);

  // メニュー操作でフォーカスが移っても選択範囲は保持されるので、focus() し直せば
  // 元の選択に対して execCommand が効く。
  const items: ContextMenuItem[] = [
    {
      label: "切り取り",
      disabled: !state.cut,
      onSelect: () =>
        run("cut", () => {
          target.focus();
          document.execCommand("cut");
        }),
    },
    {
      label: "コピー",
      disabled: !state.copy,
      onSelect: () =>
        run("copy", () => {
          target.focus();
          document.execCommand("copy");
        }),
    },
    {
      label: "貼り付け",
      disabled: !state.paste,
      onSelect: () => {
        target.focus();
        run("paste", () => paste(target));
      },
    },
    {
      label: "すべて選択",
      disabled: !state.selectAll,
      onSelect: () =>
        run("select all", () => {
          target.focus();
          selectAll(target);
        }),
    },
  ];

  // メニューは先頭の項目へフォーカスを移すので、Escapeや外側クリックで閉じた
  // あとも入力欄で続けて打てるよう、フォーカスを入力欄へ戻す。
  const handleClose = () => {
    onClose();
    target.focus();
  };

  return <ContextMenu position={position} items={items} onClose={handleClose} />;
}
