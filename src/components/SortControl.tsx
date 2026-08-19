import type { SortDirection, SortField } from "../types";

interface Props {
  field: SortField;
  direction: SortDirection;
  onChange: (field: SortField, direction: SortDirection) => void;
}

const FIELD_LABELS: Record<SortField, string> = {
  file_name: "ファイル名",
  created_at: "追加日",
  updated_date: "更新日",
  rating: "評価",
};

const FIELDS = Object.keys(FIELD_LABELS) as SortField[];

export function SortControl({ field, direction, onChange }: Props) {
  return (
    <div className="sort-control">
      <select
        value={field}
        onChange={(e) => onChange(e.target.value as SortField, direction)}
        aria-label="並び替え項目"
        data-testid="header-sort-select"
      >
        {FIELDS.map((f) => (
          <option key={f} value={f}>
            {FIELD_LABELS[f]}
          </option>
        ))}
      </select>
      <button
        type="button"
        onClick={() => onChange(field, direction === "asc" ? "desc" : "asc")}
        aria-label="並び順を切り替え"
        data-testid="header-sort-direction"
      >
        {direction === "asc" ? "昇順 ↑" : "降順 ↓"}
      </button>
    </div>
  );
}
