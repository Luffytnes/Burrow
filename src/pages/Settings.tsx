import { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Settings as SettingsIcon,
  Power,
  Fingerprint,
  Moon,
  Sun,
  MonitorCog,
  Globe,
  AlertTriangle,
  RefreshCw,
  CheckCircle2,
  ShieldAlert,
  X,
} from "lucide-react";
import { useT } from "../i18n/useT";
import { locales, LangKey } from "../i18n/locales";
import { checkMoInstalled } from "../hooks/useMo";
import { useTheme } from "../ThemeContext";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ClamavInfo {
  installed: boolean;
  version: string;
  freshclam_path: string;
}

function Toggle({ value, onChange }: { value: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      onClick={() => onChange(!value)}
      className="relative w-10 h-6 rounded-full transition-all duration-300 shrink-0"
      style={{ background: value ? "var(--accent)" : "var(--bar-track)" }}
    >
      <motion.div
        animate={{ x: value ? 18 : 2 }}
        transition={{ type: "spring", stiffness: 500, damping: 30 }}
        className="absolute top-1 w-4 h-4 rounded-full shadow shrink-0"
        style={{ background: "#ffffff" }}
      />
    </button>
  );
}

function Row({
  icon: Icon,
  label,
  desc,
  children,
}: {
  icon: React.ElementType;
  label: string;
  desc: string;
  children: React.ReactNode;
}) {
  return (
    <div className="card px-4 py-3 flex items-center gap-4">
      <Icon size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
          {label}
        </div>
        <div className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {desc}
        </div>
      </div>
      {children}
    </div>
  );
}

function UpdateButton({
  hasUpdate,
  updating,
  done,
  onUpdate,
  labelUpdate,
  labelUpdating,
  labelUpToDate,
}: {
  hasUpdate: boolean | null;
  updating: boolean;
  done: boolean;
  onUpdate: () => void;
  labelUpdate: string;
  labelUpdating: string;
  labelUpToDate: string;
}) {
  if (updating) {
    return (
      <button
        disabled
        className="flex items-center gap-1.5 text-xs font-medium px-3 py-1.5 rounded-lg opacity-60"
        style={{ background: "var(--accent)", color: "#fff" }}
      >
        <RefreshCw size={11} className="animate-spin" />
        <span>{labelUpdating}</span>
      </button>
    );
  }
  if (hasUpdate === null) {
    return <RefreshCw size={11} className="animate-spin" style={{ color: "var(--text-3)" }} />;
  }
  if (hasUpdate) {
    return (
      <button
        onClick={onUpdate}
        className="flex items-center gap-1.5 text-xs font-medium px-3 py-1.5 rounded-lg transition-all"
        style={{ background: "var(--accent)", color: "#fff" }}
      >
        <RefreshCw size={11} />
        <span>{labelUpdate}</span>
      </button>
    );
  }
  return (
    <span
      className="flex items-center gap-1.5 text-xs font-medium"
      style={{ color: "var(--success)" }}
    >
      <CheckCircle2 size={11} />
      <span>{done ? labelUpToDate : labelUpToDate}</span>
    </span>
  );
}

export default function Settings() {
  const { t, lang, setLang } = useT();
  const { mode, setMode, dark } = useTheme();
  const [startup, setStartup] = useState(false);
  const [touchId, setTouchId] = useState(false);
  const [touchIdAvailable, setTouchIdAvailable] = useState<boolean | null>(null);
  const [moInstalled, setMoInstalled] = useState<boolean | null>(null);
  const [showLangMenu, setShowLangMenu] = useState(false);
  const [moVersion, setMoVersion] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [updateDone, setUpdateDone] = useState(false);
  const [moHasUpdate, setMoHasUpdate] = useState<boolean | null>(null);

  const [clamavInfo, setClamavInfo] = useState<ClamavInfo | null>(null);
  const [clamavUpdating, setClamavUpdating] = useState(false);
  const [clamavUpdateDone, setClamavUpdateDone] = useState(false);
  const [clamavDefsOutdated, setClamavDefsOutdated] = useState<boolean | null>(null);

  const [toast, setToast] = useState<{ ok: boolean; msg: string; details?: string } | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);

  const showToast = (t: { ok: boolean; msg: string; details?: string }) => {
    setToast(t);
    setTimeout(() => setToast(null), t.ok ? 3500 : 7000);
  };

  useEffect(() => {
    checkMoInstalled()
      .then(setMoInstalled)
      .catch((e) => console.error("check_mo_installed:", e));
    invoke<string>("get_mo_version")
      .then((v) => setMoVersion(v))
      .catch((e) => console.error("get_mo_version:", e));
    invoke<ClamavInfo>("check_clamav")
      .then(setClamavInfo)
      .catch((e) => console.error("check_clamav:", e));
    invoke<boolean>("check_mo_update_available")
      .then(setMoHasUpdate)
      .catch(() => setMoHasUpdate(false));
    invoke<boolean>("check_clamav_defs_outdated")
      .then(setClamavDefsOutdated)
      .catch(() => setClamavDefsOutdated(false));
    invoke<boolean>("get_launch_at_login")
      .then(setStartup)
      .catch(() => setStartup(false));
    setTouchId(localStorage.getItem("burrow_touchid") === "true");
    invoke<boolean>("check_touch_id_available")
      .then(setTouchIdAvailable)
      .catch(() => setTouchIdAvailable(false));
  }, []);

  const handleStartupChange = async (v: boolean) => {
    setStartup(v);
    try {
      await invoke("set_launch_at_login", { enable: v });
      showToast({
        ok: true,
        msg: v ? "Lancement au démarrage activé" : "Lancement au démarrage désactivé",
      });
    } catch (err) {
      setStartup(!v);
      showToast({ ok: false, msg: `Erreur : ${err}` });
    }
  };

  const handleTouchIdChange = async (v: boolean) => {
    if (v) {
      try {
        await invoke("authenticate_touch_id", { reason: "Activer Touch ID pour Burrow" });
        // Configure pam_tid.so pour que sudo utilise Touch ID (une seule fois)
        await invoke("setup_pam_touchid");
        setTouchId(true);
        localStorage.setItem("burrow_touchid", "true");
        showToast({ ok: true, msg: "Touch ID activé" });
      } catch (err) {
        showToast({ ok: false, msg: String(err) });
      }
    } else {
      setTouchId(false);
      localStorage.removeItem("burrow_touchid");
      showToast({ ok: true, msg: "Touch ID désactivé" });
    }
  };

  const handleClamavUpdate = async () => {
    setClamavUpdating(true);
    setClamavUpdateDone(false);
    const lines: string[] = [];
    const u1 = await listen<string>("clamav-update-line", (e) => lines.push(e.payload));
    const u2 = await listen<number>("clamav-update-done", (e) => {
      setClamavUpdating(false);
      u1();
      u2();
      if (e.payload === 0) {
        setClamavUpdateDone(true);
        showToast({ ok: true, msg: "Définitions ClamAV mises à jour" });
        invoke<ClamavInfo>("check_clamav")
          .then(setClamavInfo)
          .catch((e) => console.error("check_clamav:", e));
        invoke<boolean>("check_clamav_defs_outdated")
          .then(setClamavDefsOutdated)
          .catch((e) => console.error("check_clamav_defs_outdated:", e));
      } else {
        showToast({ ok: false, msg: "Échec de la mise à jour ClamAV", details: lines.join("\n") });
      }
    });
    invoke("update_clamav_defs").catch((err) => {
      setClamavUpdating(false);
      u1();
      u2();
      showToast({ ok: false, msg: "Échec de la mise à jour ClamAV", details: String(err) });
    });
  };

  const handleUpdate = async () => {
    setUpdating(true);
    setUpdateDone(false);
    const lines: string[] = [];
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<string>("mo-output", (e) => lines.push(e.payload));
    const u2 = await listen<number>("mo-done", (e) => {
      setUpdating(false);
      u1();
      u2();
      if (e.payload === 0) {
        setUpdateDone(true);
        showToast({ ok: true, msg: "Mole CLI mis à jour" });
        invoke<boolean>("check_mo_update_available")
          .then(setMoHasUpdate)
          .catch(() => setMoHasUpdate(false));
      } else {
        showToast({ ok: false, msg: "Échec de la mise à jour Mole", details: lines.join("\n") });
      }
    });
    invoke("update_mo_cli").catch((err) => {
      setUpdating(false);
      u1();
      u2();
      showToast({ ok: false, msg: "Échec de la mise à jour Mole", details: String(err) });
    });
  };

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-4 overflow-y-auto">
      <div className="flex items-center gap-3 pt-1">
        <div
          className="w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <SettingsIcon size={16} style={{ color: "var(--text-3)" }} />
        </div>
        <div>
          <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
            {t.settings_title}
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            {t.settings_sub}
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <Row icon={Power} label={t.settings_startup} desc={t.settings_startup_desc}>
          <Toggle value={startup} onChange={handleStartupChange} />
        </Row>

        <Row icon={Fingerprint} label={t.settings_touchid} desc={t.settings_touchid_desc}>
          {touchIdAvailable === false ? (
            <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
              Non disponible
            </span>
          ) : (
            <Toggle value={touchId} onChange={handleTouchIdChange} />
          )}
        </Row>

        <Row icon={dark ? Moon : Sun} label={t.settings_theme} desc={t.settings_theme_desc}>
          <div className="segmented">
            {(
              [
                { id: "light", label: t.theme_light, icon: Sun },
                { id: "dark", label: t.theme_dark, icon: Moon },
                { id: "auto", label: t.theme_auto, icon: MonitorCog },
              ] as const
            ).map(({ id, label, icon: Icon }) => (
              <button key={id} data-active={mode === id} onClick={() => setMode(id)}>
                <Icon size={12} />
                <span>{label}</span>
              </button>
            ))}
          </div>
        </Row>

        {/* Language selector */}
        <div className="card px-4 py-3 flex items-center gap-4 relative">
          <Globe size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
              {t.settings_language}
            </div>
            <div className="text-[11px]" style={{ color: "var(--text-3)" }}>
              {t.settings_language_desc}
            </div>
          </div>
          <div className="relative">
            <button
              onClick={() => setShowLangMenu(!showLangMenu)}
              className="flex items-center gap-1.5 card rounded-full px-3 py-1.5 text-sm transition-all"
              style={{ color: "var(--text-1)" }}
            >
              <span>{locales[lang].flag}</span>
              <span>{locales[lang].label}</span>
              <span className="text-xs" style={{ color: "var(--text-3)" }}>
                ▾
              </span>
            </button>
            {showLangMenu && (
              <motion.div
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                className="absolute right-0 bottom-10 z-50 rounded-xl overflow-hidden"
                style={{
                  background: "var(--bg-card)",
                  border: "1px solid var(--border)",
                  minWidth: 140,
                  boxShadow: "0 8px 24px rgba(0,0,0,0.15)",
                }}
              >
                {(Object.keys(locales) as LangKey[]).map((key) => (
                  <button
                    key={key}
                    onClick={() => {
                      setLang(key);
                      setShowLangMenu(false);
                    }}
                    className="w-full flex items-center gap-2.5 px-4 py-2.5 text-sm transition-colors"
                    style={{
                      color: lang === key ? "var(--accent)" : "var(--text-2)",
                      background: "transparent",
                    }}
                    onMouseEnter={(e) =>
                      ((e.currentTarget as HTMLElement).style.background = "var(--bar-track)")
                    }
                    onMouseLeave={(e) =>
                      ((e.currentTarget as HTMLElement).style.background = "transparent")
                    }
                  >
                    <span>{locales[key].flag}</span>
                    <span>{locales[key].label}</span>
                    {lang === key && (
                      <span className="ml-auto" style={{ color: "var(--accent)" }}>
                        ✓
                      </span>
                    )}
                  </button>
                ))}
              </motion.div>
            )}
          </div>
        </div>
      </div>

      {/* Mole CLI */}
      <div className="flex flex-col gap-2">
        <p
          className="text-[10px] uppercase tracking-widest px-1"
          style={{ color: "var(--text-3)" }}
        >
          {t.settings_mole}
        </p>
        {moInstalled === false ? (
          <div className="card px-4 py-3" style={{ borderColor: "rgba(234,179,8,0.35)" }}>
            <div className="flex items-start gap-3">
              <AlertTriangle
                size={14}
                className="shrink-0 mt-0.5"
                style={{ color: "var(--warning-text)" }}
              />
              <div>
                <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                  {t.settings_mole_version}
                </div>
                <div className="text-[11px] font-mono" style={{ color: "var(--warning-text)" }}>
                  {t.settings_mole_missing}
                </div>
              </div>
            </div>
          </div>
        ) : (
          <div className="card px-4 py-3 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <div>
                <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                  {t.settings_mole_version}
                </div>
                <div className="text-[11px]" style={{ color: "var(--text-3)" }}>
                  {moInstalled === null ? t.settings_checking : `mo — ${moVersion ?? "..."}`}
                </div>
              </div>
              <UpdateButton
                hasUpdate={moHasUpdate}
                updating={updating}
                done={updateDone}
                onUpdate={handleUpdate}
                labelUpdate={t.settings_update}
                labelUpdating={t.settings_updating}
                labelUpToDate={t.settings_mole_uptodate}
              />
            </div>
          </div>
        )}
      </div>

      {/* ClamAV */}
      <div className="flex flex-col gap-2">
        <p
          className="text-[10px] uppercase tracking-widest px-1"
          style={{ color: "var(--text-3)" }}
        >
          {t.settings_clamav}
        </p>
        {clamavInfo?.installed === false ? (
          <div className="card px-4 py-3" style={{ borderColor: "rgba(234,179,8,0.35)" }}>
            <div className="flex items-start gap-3">
              <AlertTriangle
                size={14}
                className="shrink-0 mt-0.5"
                style={{ color: "var(--warning-text)" }}
              />
              <div>
                <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                  {t.settings_clamav_version}
                </div>
                <div className="text-[11px] font-mono" style={{ color: "var(--warning-text)" }}>
                  {t.settings_clamav_missing}
                </div>
              </div>
            </div>
          </div>
        ) : (
          <div className="card px-4 py-3 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <div>
                <div className="flex items-center gap-2">
                  <ShieldAlert size={13} style={{ color: "var(--text-3)" }} />
                  <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {t.settings_clamav_version}
                  </div>
                </div>
                <div className="text-[11px]" style={{ color: "var(--text-3)" }}>
                  {clamavInfo === null ? t.settings_checking : clamavInfo.version || "ClamAV"}
                </div>
              </div>
              <UpdateButton
                hasUpdate={clamavDefsOutdated}
                updating={clamavUpdating}
                done={clamavUpdateDone}
                onUpdate={handleClamavUpdate}
                labelUpdate={t.settings_clamav_update}
                labelUpdating={t.settings_clamav_updating}
                labelUpToDate={t.settings_mole_uptodate}
              />
            </div>
          </div>
        )}
      </div>

      <div className="mt-auto text-center text-[11px]" style={{ color: "var(--text-3)" }}>
        {t.version} <span style={{ color: "var(--text-2)" }}>Mole</span>
      </div>

      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.95 }}
            transition={{ duration: 0.2 }}
            onClick={() => toast.details && setErrorDetail(toast.details)}
            className={`fixed bottom-4 right-4 z-50 flex items-center gap-2.5 px-4 py-2.5 rounded-xl shadow-lg text-sm font-medium select-none ${toast.details ? "cursor-pointer" : ""}`}
            style={{ background: toast.ok ? "var(--success)" : "var(--danger)", color: "#fff" }}
          >
            {toast.ok ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
            {toast.msg}
            {!toast.ok && toast.details && (
              <span className="text-[11px] opacity-70 ml-1">· détails</span>
            )}
          </motion.div>
        )}
      </AnimatePresence>

      <AnimatePresence>
        {errorDetail && (
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
            transition={{ duration: 0.18 }}
            className="fixed bottom-16 right-4 z-50 rounded-xl overflow-hidden shadow-xl"
            style={{ width: 320, border: "1px solid var(--term-border)" }}
          >
            <div
              className="flex items-center justify-between px-3 py-2"
              style={{
                background: "var(--term-surface)",
                borderBottom: "1px solid var(--term-border)",
              }}
            >
              <span className="text-[11px] font-semibold" style={{ color: "var(--danger-text)" }}>
                Détails de l'erreur
              </span>
              <button onClick={() => setErrorDetail(null)} style={{ color: "var(--text-3)" }}>
                <X size={12} />
              </button>
            </div>
            <pre
              className="p-3 font-mono text-[10px] leading-relaxed overflow-y-auto max-h-48 whitespace-pre-wrap"
              style={{ background: "var(--term-bg)", color: "var(--term-text)" }}
            >
              {errorDetail}
            </pre>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
