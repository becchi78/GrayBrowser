// Pure enable/disable rules for the text-field edit menu, kept DOM-free so
// they are unit-testable under node.

export interface EditMenuInput {
  value: string;
  selectionStart: number;
  selectionEnd: number;
  readOnly: boolean;
  disabled: boolean;
  isPassword: boolean;
}

export interface EditMenuState {
  cut: boolean;
  copy: boolean;
  paste: boolean;
  selectAll: boolean;
}

// Password fields refuse cut/copy so the secret cannot reach the clipboard,
// matching the native behavior this menu replaces.
export function editMenuState(input: EditMenuInput): EditMenuState {
  const hasSelection = input.selectionEnd > input.selectionStart;
  const editable = !input.readOnly && !input.disabled;
  return {
    cut: hasSelection && editable && !input.isPassword,
    copy: hasSelection && !input.isPassword,
    paste: editable,
    selectAll: input.value.length > 0,
  };
}
