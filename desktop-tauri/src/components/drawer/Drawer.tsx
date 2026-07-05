// Drawer: slide-in panel from the right

import { ReactNode } from "react";
import { WinCloseIcon } from "../icons";

interface DrawerProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
}

export default function Drawer({ title, onClose, children }: DrawerProps) {
  return (
    <>
      <div className="drawer-overlay" onClick={onClose} />
      <div className="drawer">
        <div className="drawer__head">
          <h2 className="drawer__title">{title}</h2>
          <button className="drawer__close" onClick={onClose} aria-label="关闭">
            <WinCloseIcon />
          </button>
        </div>
        <div className="drawer__body">{children}</div>
      </div>
    </>
  );
}
