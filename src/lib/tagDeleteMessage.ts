// タグ削除の確認文言のうち、タグ名の後ろに続く部分。名前は確認ダイアログ側で
// 文中に強調して差し込むため、ここには含めない。件数の文は、外れる動画が無い
// ときに「0 件の動画から外れます」という無意味な警告にならないよう省く。
export function buildTagDeleteMessage(videoCount: number): string {
  const count =
    videoCount > 0 ? `このタグが付いている ${videoCount} 件の動画からタグが外れます。` : "";
  return `${count}元に戻せません。`;
}
