// タグ削除の確認文言。削除するタグの名前は、確認ダイアログ側で目立つ形で別に
// 出すため、ここには含めない。件数の文は、外れる動画が無いときに「0 件の動画
// から外れます」という無意味な警告にならないよう省く。
export function buildTagDeleteMessage(videoCount: number): string {
  const count =
    videoCount > 0 ? `このタグが付いている ${videoCount} 件の動画からタグが外れます。` : "";
  return `このタグを削除します。${count}元に戻せません。`;
}
