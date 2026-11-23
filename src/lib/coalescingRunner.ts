/**
 * 非同期の処理を「同時に1つだけ」走らせるための調停。
 * 実行中に届いた要求は新しく走らせず、1回分の予約にまとめる。実行が終わったら、
 * 予約があれば1回だけ続けて実行する(その時点の最新の条件で読ませたい処理向け)。
 *
 * run が reject しても調停は止まらない(予約は引き続き実行される)。
 * run の中でエラーを扱うのは呼び出し側の責任。
 */
export interface CoalescingRunner {
  /** 実行中でなければ今すぐ実行し、実行中なら終了後に1回実行する予約にする。 */
  request(): void;
  /** 以後の実行と予約を止める。実行中の処理は止めないが、終わっても再実行しない。 */
  cancel(): void;
}

export function createCoalescingRunner(run: () => Promise<unknown>): CoalescingRunner {
  let running = false;
  let pending = false;
  let cancelled = false;

  const start = () => {
    running = true;
    pending = false;
    // 同期的に例外を投げる run でも running を戻せるよう、Promise に包む。
    Promise.resolve()
      .then(run)
      .catch(() => {})
      .then(() => {
        running = false;
        if (pending && !cancelled) start();
      });
  };

  return {
    request() {
      if (cancelled) return;
      if (running) {
        pending = true;
        return;
      }
      start();
    },
    cancel() {
      cancelled = true;
      pending = false;
    },
  };
}
