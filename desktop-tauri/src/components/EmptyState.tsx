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
      <BrushMark className="empty__icon" aria-hidden="true" />
      <h3 className="empty__title">{title}</h3>
      {text ? <p className="empty__text">{text}</p> : null}
      {action ? <div className="empty__action">{action}</div> : null}
    </div>
  );
}
