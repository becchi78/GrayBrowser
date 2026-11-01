// タグ削除の確認文言。件数の文は、外れる動画が無いときに「0 件の動画から
// 外れます」という無意味な警告にならないよう省く。
export function buildTagDeleteMessage(name: string, videoCount: number): string {
  const head = `タグ『${name}』を削除します。`;
  const count =
    videoCount > 0 ? `このタグが付いている ${videoCount} 件の動画からタグが外れます。` : "";
  return `${head}${count}元に戻せません。`;
}
