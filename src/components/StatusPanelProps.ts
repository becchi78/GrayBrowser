// StatusBar のバッジから開閉する、件数つきパネル共通の Props。
export interface StatusPanelProps {
  refreshKey: number;
  // 表示の開閉は StatusBar のバッジが決め、パネル自身のボタンでは決めない。
  // 閉じていてもコンポーネントは mount したままにして、取得と購読を続けることで
  // バッジの件数を裏でも最新に保つ。
  open: boolean;
  // 現在の合計件数を StatusBar のバッジ表示のために親へ伝える。
  onCountChange?: (count: number) => void;
}
