import { Activity, Settings, ShieldCheck } from "lucide-react";
import { useT } from "../i18n/useT";

export default function Topbar({
  active,
  onSelect,
}: {
  active: string;
  onSelect: (id: string) => void;
}) {
  const { t } = useT();

  const NAV = [
    { id: "smart-scan", label: "Smart Scan" },
    { id: "dashboard", label: t.topbar_state },
    { id: "scan", label: t.topbar_scan },
    { id: "clean", label: t.topbar_clean },
    { id: "uninstall", label: t.topbar_apps },
    { id: "optimize", label: t.topbar_optimize },
  ];

  return (
    <header className="flex items-center px-6 py-2.5 shrink-0" role="banner">
      {/* Left spacer */}
      <div className="w-20 shrink-0" aria-hidden="true" />

      {/* Centered pill nav */}
      <nav aria-label="Navigation principale" className="flex-1 flex justify-center">
        <div className="nav-pill flex items-center gap-0.5 px-2 py-1.5">
          <div
            className="w-9 h-9 rounded-full overflow-hidden mr-1.5 shrink-0"
            style={{ border: "2px solid var(--border)" }}
            aria-hidden="true"
          >
            <img
              src="/logo.png"
              className="w-full h-full object-cover"
              style={{ pointerEvents: "none", transform: "scale(1.45)", transformOrigin: "center" }}
              alt=""
            />
          </div>
          {NAV.map(({ id, label }) => (
            <button
              key={id}
              onClick={() => onSelect(id)}
              aria-current={active === id ? "page" : undefined}
              className="px-4 py-1.5 rounded-full text-sm font-medium transition-all duration-150"
              style={
                active === id
                  ? { background: "var(--accent)", color: "var(--on-accent)", fontWeight: 700 }
                  : { color: "var(--text-2)" }
              }
              onMouseEnter={(e) => {
                if (active !== id) (e.currentTarget as HTMLElement).style.color = "var(--text-1)";
              }}
              onMouseLeave={(e) => {
                if (active !== id) (e.currentTarget as HTMLElement).style.color = "var(--text-2)";
              }}
            >
              {label}
            </button>
          ))}
        </div>
      </nav>

      {/* DNS + Settings icons */}
      <div className="w-24 shrink-0 flex justify-end items-center gap-0.5">
        <button
          onClick={() => onSelect("activity")}
          aria-label="Journal d’activité"
          aria-current={active === "activity" ? "page" : undefined}
          className="p-2 rounded-full transition-all"
          style={{
            color: active === "activity" ? "var(--accent)" : "var(--text-3)",
            background: active === "activity" ? "var(--accent-dim)" : "transparent",
          }}
        >
          <Activity size={14} aria-hidden="true" />
        </button>
        <button
          onClick={() => onSelect("dns")}
          aria-label="DNS Privé"
          aria-current={active === "dns" ? "page" : undefined}
          className="p-2 rounded-full transition-all"
          style={{
            color: active === "dns" ? "var(--accent)" : "var(--text-3)",
            background: active === "dns" ? "var(--accent-dim)" : "transparent",
          }}
          onMouseEnter={(e) => {
            (e.currentTarget as HTMLElement).style.color = "var(--accent)";
          }}
          onMouseLeave={(e) => {
            (e.currentTarget as HTMLElement).style.color =
              active === "dns" ? "var(--accent)" : "var(--text-3)";
          }}
        >
          <ShieldCheck size={14} aria-hidden="true" />
        </button>
        <button
          onClick={() => onSelect("settings")}
          aria-label="Réglages"
          aria-current={active === "settings" ? "page" : undefined}
          className="p-2 rounded-full transition-all"
          style={{
            color: active === "settings" ? "var(--accent)" : "var(--text-3)",
            background: active === "settings" ? "var(--accent-dim)" : "transparent",
          }}
          onMouseEnter={(e) => {
            (e.currentTarget as HTMLElement).style.color = "var(--accent)";
          }}
          onMouseLeave={(e) => {
            (e.currentTarget as HTMLElement).style.color =
              active === "settings" ? "var(--accent)" : "var(--text-3)";
          }}
        >
          <Settings size={14} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}
