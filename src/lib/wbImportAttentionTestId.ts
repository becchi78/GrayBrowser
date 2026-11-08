import type { AttentionItem } from "./wbImportResultDisplay.ts";

// Record にして、AttentionItem["key"] に項目が増えたときに、ここへの追加漏れを
// コンパイルエラーにする。undefined は、専用の data-testid を持たない項目。
const ATTENTION_TEST_IDS: Record<AttentionItem["key"], string | undefined> = {
  unmatched: "import-result-needs-review",
  "unmatched-empty-hash": "import-result-empty-hash",
  failed: undefined,
  "invalid-name": "import-result-skipped-invalid-name",
  "failed-rows": "import-result-failed-rows",
  tags: undefined,
  "skipped-roots": "import-result-skipped-roots",
  "volume-label-unresolved": "import-result-volume-label-unresolved",
};

export function attentionTestId(key: AttentionItem["key"]): string | undefined {
  return ATTENTION_TEST_IDS[key];
}
