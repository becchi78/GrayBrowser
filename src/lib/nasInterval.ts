// NAS・ネットワークドライブの確認間隔(分)の入力検証。
// 下の定数は gb_core::nas_poll_interval と同じ値でなければならない。
// 範囲の検証は常にバックエンドが正で、ここは不正値を送る前に理由を
// 即時に見せるための補助にすぎない(片方だけを変更しないこと)。
export const MIN_MINUTES = 1;
export const MAX_MINUTES = 1440;
export const DEFAULT_MINUTES = 10;

export type ParseIntervalResult = { ok: true; minutes: number } | { ok: false; reason: string };

export function parseIntervalMinutes(text: string): ParseIntervalResult {
  const trimmed = text.trim();
  if (trimmed === "") {
    return { ok: false, reason: "確認間隔を入力してください。" };
  }
  // Number() は "1e3" や "0x10"、小数も通してしまうため、ASCII の数字だけを許す。
  if (!/^[0-9]+$/.test(trimmed)) {
    return { ok: false, reason: "確認間隔は半角数字の整数で入力してください。" };
  }
  const minutes = Number(trimmed);
  if (minutes < MIN_MINUTES || minutes > MAX_MINUTES) {
    return { ok: false, reason: `確認間隔は${MIN_MINUTES}〜${MAX_MINUTES}分の範囲で入力してください。` };
  }
  return { ok: true, minutes };
}
