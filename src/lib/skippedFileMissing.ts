// 元のファイルが見つからない未登録行の表示まわりの純関数。

export const MISSING_FILE_NOTE = "元のファイルが見つかりません";

export const MISSING_FILE_GUIDANCE =
  "保管先に移したファイルは、名前を直して登録すると、オフラインの動画として記録が残ります。";

// 登録後の .wb 再インポートでは補完できない(登録済みの動画は上書きしない)ため、
// 「登録の前に」と案内する。
export function missingRegistrationNotice(carriedWbData: boolean): string | null {
  return carriedWbData
    ? null
    : "オフラインの動画として登録しました。この記録には.wbの情報がないため、タグ・評価は付いていません。タグなどを引き継ぎたい場合は、登録の前に.wbをインポートし直してください。";
}
