/** invoke の拒否値(文字列)も Error も、利用者に見せる一行にする。 */
export function errorText(e: unknown): string {
  if (typeof e === "string") {
    return e;
  }
  return e instanceof Error ? e.message : String(e);
}
