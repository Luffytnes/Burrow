import { useState, useEffect, useCallback } from "react";
import { motion } from "framer-motion";
import { ShieldCheck, ExternalLink, RefreshCw, HardDrive } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n/useT";

export default function PermissionsGate({ children }: { children: React.ReactNode }) {
  const { t } = useT();
  const [status, setStatus] = useState<"checking" | "granted" | "denied">("checking");

  const check = useCallback(async () => {
    setStatus("checking");
    try {
      const ok = await invoke<boolean>("check_full_disk_access");
      setStatus(ok ? "granted" : "denied");
    } catch {
      // Command unavailable or error → show the screen to be safe
      setStatus("denied");
    }
  }, []);

  useEffect(() => {
    check();
  }, [check]);

  const openSettings = async () => {
    await invoke("open_full_disk_access_settings").catch((e) =>
      console.error("open_full_disk_access_settings:", e)
    );
  };

  if (status === "checking") {
    return (
      <div
        className="flex items-center justify-center h-full gap-2"
        style={{ color: "var(--text-3)" }}
      >
        <RefreshCw size={13} className="animate-spin" />
        <span className="text-xs">{t.perm_checking}</span>
      </div>
    );
  }

  if (status === "granted") return <>{children}</>;

  return (
    <div className="flex flex-col items-center justify-center h-full gap-6 px-10">
      <motion.div
        initial={{ scale: 0.85, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        transition={{ duration: 0.3 }}
        className="w-20 h-20 rounded-2xl flex items-center justify-center"
        style={{ background: "var(--accent-dim)", border: "1px solid var(--border)" }}
      >
        <HardDrive size={36} style={{ color: "var(--accent)" }} />
      </motion.div>

      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.1 }}
        className="text-center max-w-sm space-y-3"
      >
        <h2 className="text-xl font-bold" style={{ color: "var(--text-1)" }}>
          {t.perm_title}
        </h2>
        <p className="text-sm" style={{ color: "var(--text-2)" }}>
          {t.perm_sub}
        </p>
        <p className="text-xs leading-relaxed" style={{ color: "var(--text-3)" }}>
          {t.perm_why}
        </p>
      </motion.div>

      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.18 }}
        className="flex flex-col items-center gap-3"
      >
        <button onClick={openSettings} className="btn-primary flex items-center gap-2">
          <ExternalLink size={14} />
          {t.perm_btn}
        </button>
        <button
          onClick={check}
          className="flex items-center gap-1.5 text-xs transition-colors"
          style={{ color: "var(--text-3)" }}
          onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.color = "var(--accent)")}
          onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.color = "var(--text-3)")}
        >
          <ShieldCheck size={12} />
          {t.perm_check}
        </button>
      </motion.div>
    </div>
  );
}
