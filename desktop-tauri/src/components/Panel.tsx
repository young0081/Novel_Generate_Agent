// The shared paper panel that wraps every screen, with a serif title, an
// ink underline flourish, an optional English subtitle, and a corner brush.

import type { ReactNode } from "react";
import { BrushStroke, TitleStroke } from "./icons";

interface PanelProps {
  title: string;
  en?: string;
  subtitle?: string;
  actions?: ReactNode;
  children: ReactNode;
  /** Extra content rendered between the header and the body (e.g. toolbar). */
  toolbar?: ReactNode;
}

export default function Panel({
  title,
  en,
  subtitle,
  actions,
  toolbar,
  children,
}: PanelProps) {
  return (
    <section className="panel">
      <BrushStroke className="panel__flourish" aria-hidden="true" />
      <header className="panel__header">
        <div className="panel__heading">
          <h2 className="panel__title">
            <span className="panel__title-cn">{title}</span>
            {en ? <span className="en">{en}</span> : null}
            <TitleStroke className="panel__title-stroke" aria-hidden="true" />
          </h2>
          {subtitle ? <p className="panel__subtitle">{subtitle}</p> : null}
        </div>
        {actions ? <div className="panel__actions">{actions}</div> : null}
      </header>
      {toolbar && <div className="panel__toolbar">{toolbar}</div>}
      <div className="panel__body">{children}</div>
    </section>
  );
}
