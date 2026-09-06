"use client";

import Button from "@/components/Button";

interface BatchActionsProps {
  selectedCount: number;
  totalCount: number;
  allSelected: boolean;
  busy?: boolean;
  progress?: { done: number; total: number } | null;
  onToggleAll: () => void;
  onClear: () => void;
  onDelete: () => void;
  label?: string;
}

/** Shared selection toolbar for lists with destructive bulk operations. */
export default function BatchActions({
  selectedCount,
  totalCount,
  allSelected,
  busy = false,
  progress = null,
  onToggleAll,
  onClear,
  onDelete,
  label = "批量管理",
}: BatchActionsProps) {
  return (
    <div className="batch-actions" role="toolbar" aria-label={label}>
      <span className="batch-actions__count" aria-live="polite">
        已选 <strong>{selectedCount}</strong> / {totalCount}
      </span>
      <div className="batch-actions__buttons">
        <Button variant="ghost" onClick={onToggleAll} disabled={busy}>
          {allSelected ? "取消全选" : "全选当前"}
        </Button>
        <Button variant="ghost" onClick={onClear} disabled={busy || selectedCount === 0}>
          清除
        </Button>
        <Button variant="danger" onClick={onDelete} disabled={busy || selectedCount === 0} loading={busy}>
          删除已选
        </Button>
      </div>
      {progress && (
        <div className="batch-actions__progress" aria-live="polite">
          <span>正在处理 {progress.done} / {progress.total}</span>
          <progress value={progress.done} max={progress.total || 1} />
        </div>
      )}
    </div>
  );
}
