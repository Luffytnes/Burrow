import { motion } from "framer-motion";
import {
  Sparkles,
  Trash2,
  HardDrive,
  Zap,
  PackageX,
  Archive,
  AppWindow,
  Settings,
  ShieldAlert,
} from "lucide-react";
import { useT } from "../i18n/useT";

export default function Sidebar({
  active,
  onSelect,
}: {
  active: string;
  onSelect: (id: string) => void;
}) {
  const { t } = useT();

  const nav = [
    { id: "dashboard", label: t.nav_status, icon: Sparkles },
    { id: "scan", label: t.nav_scan, icon: ShieldAlert },
    { id: "clean", label: t.nav_clean, icon: Trash2 },
    { id: "analyze", label: t.nav_storage, icon: HardDrive },
    { id: "optimize", label: t.nav_optimize, icon: Zap },
    { id: "purge", label: t.nav_purge, icon: PackageX },
    { id: "installer", label: t.nav_installers, icon: Archive },
    { id: "uninstall", label: t.nav_uninstall, icon: AppWindow },
  ];

  const activeStyle = {
    background: "linear-gradient(135deg, rgba(127,29,29,0.12), rgba(255,0,0,0.08))",
    border: "1px solid rgba(255,0,0,0.20)",
  };

  return (
    <aside
      className="w-52 flex flex-col py-6 px-3 shrink-0 border-r"
      style={{ background: "var(--bg-sidebar)", borderColor: "var(--border)" }}
    >
      <div className="flex items-center gap-2.5 px-3 mb-8">
        <img src="/logo.png" className="w-8 h-8 rounded-xl object-cover" alt="Burrow" />
        <span className="font-semibold text-base tracking-tight" style={{ color: "var(--text-1)" }}>
          Burrow
        </span>
      </div>

      <nav className="flex flex-col gap-0.5 flex-1">
        {nav.map(({ id, label, icon: Icon }) => {
          const isActive = active === id;
          return (
            <motion.button
              key={id}
              onClick={() => onSelect(id)}
              whileTap={{ scale: 0.97 }}
              className="flex items-center gap-3 px-3 py-2.5 rounded-xl text-sm font-medium transition-all duration-150 text-left w-full"
              style={
                isActive ? { ...activeStyle, color: "var(--accent)" } : { color: "var(--text-2)" }
              }
              onMouseEnter={(e) => {
                if (!isActive) (e.currentTarget as HTMLElement).style.background = "var(--bg-card)";
              }}
              onMouseLeave={(e) => {
                if (!isActive) (e.currentTarget as HTMLElement).style.background = "transparent";
              }}
            >
              <Icon size={16} style={{ color: isActive ? "var(--accent)" : "var(--text-3)" }} />
              {label}
            </motion.button>
          );
        })}
      </nav>

      <motion.button
        onClick={() => onSelect("settings")}
        whileTap={{ scale: 0.97 }}
        className="flex items-center gap-3 px-3 py-2.5 rounded-xl text-sm font-medium transition-all w-full"
        style={
          active === "settings"
            ? { ...activeStyle, color: "var(--accent)" }
            : { color: "var(--text-2)" }
        }
        onMouseEnter={(e) => {
          if (active !== "settings")
            (e.currentTarget as HTMLElement).style.background = "var(--bg-card)";
        }}
        onMouseLeave={(e) => {
          if (active !== "settings")
            (e.currentTarget as HTMLElement).style.background = "transparent";
        }}
      >
        <Settings
          size={16}
          style={{ color: active === "settings" ? "var(--accent)" : "var(--text-3)" }}
        />
        {t.nav_settings}
      </motion.button>
    </aside>
  );
}
