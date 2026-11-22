// 開発者モードの設定が保存されたことを、設定ダイアログから常時表示のバッジへ伝える。
// 両者は別の枝にあり、App を経由して状態を持ち回ると App.tsx の変更が大きくなるため、
// window のイベントで疎結合にする。
export const DEVELOPER_MODE_CHANGED_EVENT = "gb:developer-mode-changed";
