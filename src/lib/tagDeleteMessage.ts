// タグ削除の確認文言のうち、タグ名の後ろに続く部分。名前は確認ダイアログ側で
// 文中に強調して差し込むため、ここには含めない。件数の文は、外れる動画が無い
// ときに「0 件の動画から外れます」という無意味な警告にならないよう省く。
// 子のタグの文も同様に、子がいるときだけ付ける(子は消えず、親なしに戻る)。
export function buildTagDeleteMessage(videoCount: number, childCount = 0): string {
  const count =
    videoCount > 0 ? `このタグが付いている ${videoCount} 件の動画からタグが外れます。` : "";
  const children =
    childCount > 0 ? `子のタグ ${childCount} 件は削除されず、親なしに戻ります。` : "";
  return `${count}${children}元に戻せません。`;
}
