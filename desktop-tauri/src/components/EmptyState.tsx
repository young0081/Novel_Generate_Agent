import type { ReactNode } from "react";
import { BrushMark } from "./icons";

interface EmptyStateProps {
  title: string;
  text?: string;
  action?: ReactNode;
}

export default function EmptyState({ title, text, action }: EmptyStateProps) {
  return (
    <div className="empty">
      <BrushMark className="empty__icon" />
      <div className="empty__title">{title}</div>
      {text ? <p className="empty__text">{text}</p> : null}
      {action}
    </div>
  );
}
