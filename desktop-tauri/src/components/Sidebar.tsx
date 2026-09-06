// Left navigation with an ink-brush active indicator.

import { SCREENS, type ScreenId } from "../lib/screens";

interface SidebarProps {
  active: ScreenId;
  onSelect: (id: ScreenId) => void;
}

export default function Sidebar({ active, onSelect }: SidebarProps) {
  return (
    <nav className="sidebar" aria-label="主导航">
      <div className="sidebar__head">
        <img className="brand-mark" src="/icon.png" alt="" aria-hidden="true" />
        <div>
          <div className="sidebar__title">创作工坊</div>
          <div className="sidebar__caption">ATELIER</div>
        </div>
      </div>

      <div className="nav__label">书房</div>
      <div className="nav">
        {SCREENS.map((s) => {
          const Icon = s.icon;
          const isActive = s.id === active;
          return (
            <button
              key={s.id}
              className={`nav-item${isActive ? " is-active" : ""}`}
              onClick={() => onSelect(s.id)}
              aria-current={isActive ? "page" : undefined}
            >
              <span className="nav-item__icon">
                <Icon size={18} />
              </span>
              <span className="nav-item__label">{s.label}</span>
              <span className="nav-item__hint">{s.hint}</span>
            </button>
          );
        })}
      </div>

      <div className="sidebar__foot">
        <strong>墨 · 创作</strong>
        <br />
        以文载道，以墨传情。
      </div>
    </nav>
  );
}
