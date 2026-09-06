import type { ReactNode } from "react";

interface BatchActionsProps {
  selectedCount: number;
  totalCount: number;
  allSelected: boolean;
  onToggleAll: () => void;
  onClear: () => void;
  children?: ReactNode;
  label?: string;
}
/** Shared selection toolbar for lists that support safe bulk operations. */
export default function BatchActions({
  selectedCount,
  totalCount,
  allSelected,
  onToggleAll,
  onClear,
  children,
  label = "批量管理",
}: BatchActionsProps) {
  return (
    <div className="batch-actions" role="toolbar" aria-label={label}>
      <span className="batch-actions__count" aria-live="polite">
        已选 <strong>{selectedCount}</strong> / {totalCount}
      </span>
      <button type="button" className="batch-actions__link" onClick={onToggleAll}>
        {allSelected ? "取消全选" : "全选当前"}
      </button>
      <button
        type="button"
        className="batch-actions__link"
        onClick={onClear}
        disabled={selectedCount === 0}
      >
        清除
      </button>
      {children}
    </div>
  );
}
