import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { flushSync } from "react-dom";
import { motion, AnimatePresence } from "framer-motion";
import {
  Search,
  Trash2,
  ChevronDown,
  Loader2,
  RefreshCw,
  X,
  CheckCircle2,
  AlertCircle,
  Download,
  Package,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useMo } from "../hooks/useMo";
import { useT } from "../i18n/useT";

// ── Types ─────────────────────────────────────────────────────────────────────

interface AppInfo {
  name: string;
  path: string;
  size_mb: number;
}
interface FileEntry {
  name: string;
  path: string;
  size_bytes: number;
  is_dir: boolean;
}
interface BrewOutdated {
  name: string;
  installed_version: string;
  current_version: string;
  download_url: string;
  brew_managed: boolean;
}
interface UpToDateApp {
  name: string;
  current_version: string;
}
interface BrewResult {
  updates: BrewOutdated[];
  up_to_date: UpToDateApp[];
  up_to_date_cask: UpToDateApp[];
  checked: number;
}
interface BrewFormulaResult {
  updates: BrewOutdated[];
  up_to_date: UpToDateApp[];
  checked: number;
}
interface SparkleUpdate {
  name: string;
  path: string;
  current_version: string;
  latest_version: string;
  download_url: string;
  release_notes: string;
}
interface SparkleResult {
  updates: SparkleUpdate[];
  up_to_date: UpToDateApp[];
  checked: number;
}
interface AppStoreUpdate {
  name: string;
  bundle_id: string;
  installed_version: string;
  latest_version: string;
  release_notes: string;
  store_url: string;
  track_id: number;
  from_store: boolean;
}
interface AppStoreResult {
  updates: AppStoreUpdate[];
  up_to_date: UpToDateApp[];
  checked: number;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const sizeCache = new Map<string, number>();
const residualCache = new Map<string, FileEntry[]>();

function formatBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} Go`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(0)} Mo`;
  if (b >= 1e3) return `${(b / 1e3).toFixed(0)} Ko`;
  return `${b} o`;
}
function formatSize(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} Go`;
  if (mb > 0) return `${mb} Mo`;
  return "—";
}

function findIconSrc(name: string, icons: Record<string, string>): string | undefined {
  if (icons[name]) return icons[name];
  const norm = name.replace(/-/g, " ").toLowerCase();
  const key = Object.keys(icons).find((k) => k.toLowerCase() === norm);
  return key ? icons[key] : undefined;
}

function AppIcon({
  name,
  icons,
  size = 8,
}: {
  name: string;
  icons: Record<string, string>;
  size?: number;
}) {
  const src = findIconSrc(name, icons);
  const cls = `w-${size} h-${size} rounded-lg object-cover shrink-0`;
  if (src) return <img src={src} className={cls} alt={name} />;
  return (
    <div
      className={`w-${size} h-${size} rounded-lg flex items-center justify-center text-xs font-bold shrink-0`}
      style={{ background: "var(--accent-dim)", color: "var(--accent-text)" }}
    >
      {name[0]?.toUpperCase()}
    </div>
  );
}

type SortKey = "name" | "size";

// ── Section header with optional per-section "update all" ─────────────────────

function SectionHeader({
  label,
  count,
  loading,
  onUpdateAll,
  pendingCount,
}: {
  label: string;
  count?: number;
  loading?: boolean;
  onUpdateAll?: () => void;
  pendingCount?: number;
}) {
  return (
    <div className="flex items-center gap-2 px-0.5">
      <span
        className="text-[10px] uppercase tracking-widest font-semibold"
        style={{ color: "var(--text-3)" }}
      >
        {label}
      </span>
      {loading && <Loader2 size={10} className="animate-spin" style={{ color: "var(--text-3)" }} />}
      {!loading && count !== undefined && (
        <span
          className="text-[10px] px-1.5 py-0.5 rounded-full font-semibold"
          style={{
            background: count > 0 ? "var(--accent-dim)" : "var(--bar-track)",
            color: count > 0 ? "var(--accent-text)" : "var(--text-3)",
          }}
        >
          {count}
        </span>
      )}
      {!loading && onUpdateAll && (pendingCount ?? 0) >= 1 && (
        <button
          onClick={onUpdateAll}
          className="ml-auto text-[10px] font-medium px-2 py-0.5 rounded-full transition-colors"
          style={{ background: "var(--accent-dim)", color: "var(--accent-text)" }}
        >
          Tout ({pendingCount})
        </button>
      )}
    </div>
  );
}

// ── Compact update row ─────────────────────────────────────────────────────────

function UpdateRow({
  name,
  from,
  to,
  upToDate,
  notes,
  onUpdate,
  busy,
  icons,
  actionLabel,
}: {
  name: string;
  from: string;
  to?: string;
  upToDate?: boolean;
  notes?: string;
  onUpdate?: () => void;
  busy?: boolean;
  icons: Record<string, string>;
  actionLabel?: string;
}) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="card px-3 py-2 flex flex-col gap-1.5" style={{ opacity: upToDate ? 0.5 : 1 }}>
      <div className="flex items-center gap-2.5">
        <AppIcon name={name} icons={icons} size={7} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-xs font-medium truncate" style={{ color: "var(--text-1)" }}>
              {name}
            </span>
            {!upToDate && notes && (
              <button
                onClick={() => setExpanded((v) => !v)}
                className="text-[9px] px-1 py-0.5 rounded shrink-0"
                style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
              >
                {expanded ? "▲" : "▼"}
              </button>
            )}
          </div>
          <div
            className="text-[10px] font-mono leading-none mt-0.5"
            style={{ color: "var(--text-3)" }}
          >
            {upToDate ? (
              from
            ) : (
              <>
                {from} <span style={{ color: "var(--accent)" }}>→</span> {to}
              </>
            )}
          </div>
        </div>
        {upToDate ? (
          <span className="text-[10px] font-semibold shrink-0" style={{ color: "var(--success)" }}>
            {t.uninstall_up_to_date}
          </span>
        ) : (
          <button
            onClick={onUpdate}
            disabled={busy}
            className="flex items-center gap-1 btn-primary py-1 px-2.5 text-[10px] disabled:opacity-50 shrink-0"
          >
            {busy ? <Loader2 size={10} className="animate-spin" /> : <Download size={10} />}
            {actionLabel ?? t.uninstall_update}
          </button>
        )}
      </div>
      {expanded && notes && (
        <div
          className="text-[10px] px-1 whitespace-pre-wrap leading-relaxed"
          style={{ color: "var(--text-2)", borderTop: "1px solid var(--border)", paddingTop: 6 }}
        >
          {notes}
        </div>
      )}
    </div>
  );
}

// ── Tab: Désinstaller ─────────────────────────────────────────────────────────

function UninstallTab({
  icons,
  onUninstalled,
  refreshKey,
}: {
  icons: Record<string, string>;
  onUninstalled: (name: string) => void;
  refreshKey?: number;
}) {
  const { t } = useT();
  const mo = useMo();
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [activeApp, setActiveApp] = useState<AppInfo | null>(null);
  const [toast, setToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const [loadingSizes, setLoadingSizes] = useState<Set<string>>(new Set());
  const [residuals, setResiduals] = useState<Map<string, FileEntry[]>>(new Map());
  const [loadingResiduals, setLoadingResiduals] = useState<Set<string>>(new Set());
  const [sortKey, setSortKey] = useState<SortKey>("name");

  const loadApps = useCallback(() => {
    setLoading(true);
    invoke<AppInfo[]>("list_apps")
      .then(setApps)
      .catch(() => setApps([]))
      .finally(() => setLoading(false));
  }, []);
  useEffect(() => {
    loadApps();
  }, [loadApps]);
  useEffect(() => {
    if (refreshKey) loadApps();
  }, [refreshKey, loadApps]);

  const handleExpand = async (app: AppInfo) => {
    const p = app.path;
    setExpanded(expanded === p ? null : p);
    if (expanded === p) return;
    if (!sizeCache.has(p)) {
      setLoadingSizes((prev) => new Set(prev).add(p));
      invoke<number>("get_app_size", { appPath: p })
        .then((size) => {
          sizeCache.set(p, size);
          setApps((prev) => prev.map((a) => (a.path === p ? { ...a, size_mb: size } : a)));
        })
        .catch((e) => console.error("get_app_size:", e))
        .finally(() =>
          setLoadingSizes((prev) => {
            const n = new Set(prev);
            n.delete(p);
            return n;
          })
        );
    }
    if (!residualCache.has(p)) {
      setLoadingResiduals((prev) => new Set(prev).add(p));
      invoke<FileEntry[]>("find_app_residuals", { appName: app.name, appPath: app.path })
        .then((files) => {
          residualCache.set(p, files);
          setResiduals((prev) => new Map(prev).set(p, files));
        })
        .catch(() => {
          residualCache.set(p, []);
        })
        .finally(() =>
          setLoadingResiduals((prev) => {
            const n = new Set(prev);
            n.delete(p);
            return n;
          })
        );
    }
  };

  const handleUninstall = async (app: AppInfo) => {
    setActiveApp(app);
    setExpanded(null);
    mo.reset();
    const code = await mo.uninstall(app.name, app.path);
    if (code === 0) {
      setApps((prev) => prev.filter((a) => a.path !== app.path));
      onUninstalled(app.name);
      setToast({ ok: true, msg: `${app.name} désinstallé` });
    } else {
      setToast({ ok: false, msg: `Échec de la désinstallation de ${app.name}` });
    }
    setActiveApp(null);
    setTimeout(() => setToast(null), 3500);
  };

  const isRunning = mo.status === "running";

  const filtered = apps
    .filter((a) => a.name.toLowerCase().includes(query.toLowerCase()))
    .sort((a, b) =>
      sortKey === "name" ? a.name.localeCompare(b.name) : (b.size_mb || 0) - (a.size_mb || 0)
    );

  return (
    <div className="flex flex-col h-full gap-2.5 relative">
      {/* Sort + search */}
      <div className="flex items-center gap-2">
        <div className="flex items-center gap-0.5 text-[10px]">
          {(["name", "size"] as SortKey[]).map((k) => (
            <button
              key={k}
              onClick={() => setSortKey(k)}
              className="px-2.5 py-1 rounded-full transition-colors"
              style={{
                color: sortKey === k ? "var(--text-1)" : "var(--text-3)",
                background: sortKey === k ? "var(--bar-track)" : "transparent",
                fontWeight: sortKey === k ? 600 : 400,
              }}
            >
              {k === "name" ? t.uninstall_sort_name : t.uninstall_sort_size}
            </button>
          ))}
        </div>
        <div className="flex-1 card flex items-center gap-2 px-2.5 py-1.5">
          <Search size={12} style={{ color: "var(--text-3)", flexShrink: 0 }} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t.uninstall_search_placeholder}
            className="flex-1 bg-transparent text-xs outline-none"
            style={{ color: "var(--text-1)" }}
          />
          {query && (
            <button onClick={() => setQuery("")} style={{ color: "var(--text-3)" }}>
              <X size={10} />
            </button>
          )}
        </div>
        <button
          onClick={loadApps}
          disabled={loading}
          className="p-1.5 transition-colors disabled:opacity-30"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div
            className="flex items-center justify-center h-full gap-2"
            style={{ color: "var(--text-3)" }}
          >
            <Loader2 size={14} className="animate-spin" />
            <span className="text-xs">{t.uninstall_scanning_apps}</span>
          </div>
        ) : (
          <div className="flex flex-col gap-1">
            {filtered.map((app, i) => (
              <motion.div
                key={app.path}
                initial={{ opacity: 0, y: 3 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: Math.min(i * 0.02, 0.2) }}
                className="card overflow-hidden"
              >
                <button
                  onClick={() => handleExpand(app)}
                  className="w-full px-3 py-2 flex items-center gap-2.5 text-left transition-colors"
                  onMouseEnter={(e) =>
                    ((e.currentTarget as HTMLElement).style.background = "var(--bg)")
                  }
                  onMouseLeave={(e) =>
                    ((e.currentTarget as HTMLElement).style.background = "transparent")
                  }
                >
                  <AppIcon name={app.name} icons={icons} size={8} />
                  <div className="flex-1 min-w-0">
                    <div
                      className="text-xs font-medium truncate"
                      style={{ color: "var(--text-1)" }}
                    >
                      {app.name}
                    </div>
                    <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                      {app.path}
                    </div>
                  </div>
                  <span
                    className="text-xs font-semibold shrink-0"
                    style={{ color: "var(--text-2)" }}
                  >
                    {loadingSizes.has(app.path) ? (
                      <Loader2 size={10} className="animate-spin" />
                    ) : (
                      formatSize(app.size_mb)
                    )}
                  </span>
                  <ChevronDown
                    size={12}
                    className={`shrink-0 transition-transform ${expanded === app.path ? "rotate-180" : ""}`}
                    style={{ color: "var(--text-3)" }}
                  />
                </button>
                <AnimatePresence>
                  {expanded === app.path && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: "auto", opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      className="overflow-hidden"
                    >
                      <div
                        className="px-3 pb-2.5 pt-2 flex flex-col gap-1.5"
                        style={{ borderTop: "1px solid var(--border)" }}
                      >
                        <div className="flex items-center gap-1.5">
                          <span
                            className="text-[9px] px-1.5 py-0.5 rounded font-mono shrink-0"
                            style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                          >
                            .app
                          </span>
                          <span
                            className="text-[9px] font-mono truncate"
                            style={{ color: "var(--text-3)" }}
                          >
                            {app.path}
                          </span>
                        </div>
                        {loadingResiduals.has(app.path) ? (
                          <div
                            className="flex items-center gap-1 text-[10px]"
                            style={{ color: "var(--text-3)" }}
                          >
                            <Loader2 size={10} className="animate-spin" />{" "}
                            {t.uninstall_searching_residuals}
                          </div>
                        ) : (residuals.get(app.path) ?? []).length > 0 ? (
                          <div className="flex flex-col gap-0.5">
                            <div
                              className="text-[9px] uppercase tracking-widest"
                              style={{ color: "var(--text-3)" }}
                            >
                              {t.uninstall_residuals_title}
                            </div>
                            {(residuals.get(app.path) ?? []).map((f) => (
                              <div key={f.path} className="flex items-center gap-2 text-[10px]">
                                <span
                                  className="flex-1 truncate font-mono"
                                  style={{ color: "var(--text-2)" }}
                                >
                                  {f.name}
                                </span>
                                <span style={{ color: "var(--text-3)" }}>
                                  {formatBytes(f.size_bytes)}
                                </span>
                              </div>
                            ))}
                          </div>
                        ) : (
                          <div className="text-[10px]" style={{ color: "var(--success)" }}>
                            {t.uninstall_no_residuals}
                          </div>
                        )}
                        <button
                          onClick={() => handleUninstall(app)}
                          disabled={isRunning}
                          className="btn-primary flex items-center justify-center gap-1.5 py-1.5 text-xs disabled:opacity-50"
                        >
                          <Trash2 size={11} />
                          {t.uninstall_btn.replace("{{name}}", app.name)}
                        </button>
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </motion.div>
            ))}
            {filtered.length === 0 && !loading && (
              <div className="text-center py-10 text-xs" style={{ color: "var(--text-3)" }}>
                {t.uninstall_none_found}
              </div>
            )}
          </div>
        )}
      </div>

      {!loading && apps.length > 0 && (
        <div className="text-[10px] text-center" style={{ color: "var(--text-3)" }}>
          {t.uninstall_apps_count
            .replace("{n}", String(filtered.length))
            .replace("{s}", filtered.length !== 1 ? "s" : "")}
        </div>
      )}

      {/* Loading overlay */}
      {isRunning && activeApp && (
        <div
          className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 rounded-xl"
          style={{
            background: "var(--bg)",
            backdropFilter: "blur(4px)",
            WebkitBackdropFilter: "blur(4px)",
          }}
        >
          <Loader2 size={24} className="animate-spin" style={{ color: "var(--accent)" }} />
          <span className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
            Désinstallation de {activeApp.name}…
          </span>
        </div>
      )}

      {/* Toast bottom-right */}
      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.95 }}
            transition={{ duration: 0.2 }}
            className="fixed bottom-4 right-4 z-50 flex items-center gap-2.5 px-4 py-2.5 rounded-xl shadow-lg text-sm font-medium"
            style={{ background: toast.ok ? "var(--success)" : "var(--danger)", color: "#fff" }}
          >
            {toast.ok ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
            {toast.msg}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ── Tab: Mises à jour ─────────────────────────────────────────────────────────

function UpdatesTab({
  icons,
  uninstalledNames,
}: {
  icons: Record<string, string>;
  uninstalledNames: Set<string>;
}) {
  const { t } = useT();

  // ── Homebrew casks ──────────────────────────────────────────────────────────
  const [brewApps, setBrewApps] = useState<BrewOutdated[]>([]);
  const [brewUpToDate, setBrewUpToDate] = useState<UpToDateApp[]>([]);
  const [caskUpToDate, setCaskUpToDate] = useState<UpToDateApp[]>([]);
  const [brewLoading, setBrewLoading] = useState(true);
  const [brewCheckedCount, setBrewCheckedCount] = useState(0);
  const [brewBusy, setBrewBusy] = useState<string | null>(null);

  // ── Sparkle ─────────────────────────────────────────────────────────────────
  const [sparkleApps, setSparkleApps] = useState<SparkleUpdate[]>([]);
  const [sparkleUpToDate, setSparkleUpToDate] = useState<UpToDateApp[]>([]);
  const [sparkleLoading, setSparkleLoading] = useState(false);
  const [sparkleChecked, setSparkleChecked] = useState(false);
  const [sparkleBusy, setSparkleBusy] = useState<string | null>(null);

  // ── App Store ────────────────────────────────────────────────────────────────
  const [masApps, setMasApps] = useState<AppStoreUpdate[]>([]);
  const [masUpToDate, setMasUpToDate] = useState<UpToDateApp[]>([]);
  const [masLoading, setMasLoading] = useState(true);
  const [masCheckedCount, setMasCheckedCount] = useState(0);
  const [masBusy, setMasBusy] = useState<string | null>(null);

  // ── Toast + détails erreur ───────────────────────────────────────────────────
  const [toast, setToast] = useState<{ ok: boolean; msg: string; details?: string } | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);

  const showToast = useCallback((t: { ok: boolean; msg: string; details?: string }) => {
    setToast(t);
    setTimeout(() => setToast(null), t.ok ? 3500 : 7000);
  }, []);

  // ── Loading items (style Dashboard) ─────────────────────────────────────────
  const [loadingItems, setLoadingItems] = useState([
    { id: "brew", label: "Homebrew (casks & formules)", done: false },
    { id: "sparkle", label: "Mises à jour in-app (Sparkle)", done: false },
    { id: "mas", label: "App Store", done: false },
  ]);
  const markDone = (id: string) =>
    setLoadingItems((p) => p.map((i) => (i.id === id ? { ...i, done: true } : i)));

  const checkSparkle = useCallback(async () => {
    setSparkleLoading(true);
    setSparkleChecked(false);
    setSparkleApps([]);
    setSparkleUpToDate([]);
    try {
      const res = await invoke<SparkleResult>("check_sparkle_updates");
      setSparkleApps(res.updates);
      setSparkleUpToDate(res.up_to_date);
      setSparkleChecked(true);
    } catch (e) {
      console.error("check_sparkle_updates:", e);
    } finally {
      setSparkleLoading(false);
      setLoadingItems((p) => p.map((i) => (i.id === "sparkle" ? { ...i, done: true } : i)));
    }
  }, []);

  useEffect(() => {
    invoke<BrewResult>("get_brew_outdated")
      .then((r) => {
        setBrewApps(r.updates);
        setBrewUpToDate(r.up_to_date);
        setCaskUpToDate(r.up_to_date_cask);
        setBrewCheckedCount(r.checked);
        setBrewLoading(false);
        markDone("brew");
      })
      .catch(() => {
        setBrewLoading(false);
        markDone("brew");
      });
    checkSparkle();
    invoke<AppStoreResult>("check_app_store_updates")
      .then((r) => {
        setMasApps(r.updates);
        setMasUpToDate(r.up_to_date);
        setMasCheckedCount(r.checked);
        setMasLoading(false);
        markDone("mas");
      })
      .catch(() => {
        setMasLoading(false);
        markDone("mas");
      });
  }, [checkSparkle]);

  // ── Handlers ─────────────────────────────────────────────────────────────────

  const handleBrewUpdate = async (appName: string, brewManaged: boolean, downloadUrl: string) => {
    setBrewBusy(appName);
    const lines: string[] = [];
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<string>("mo-output", (e) => lines.push(e.payload));
    const u2 = await listen<number>("mo-done", (e) => {
      setBrewBusy(null);
      u1();
      u2();
      if (e.payload === 0) {
        setBrewApps((prev) => {
          const app = prev.find((a) => a.name === appName);
          if (app)
            setBrewUpToDate((u) => [
              ...u,
              { name: app.name, current_version: app.current_version },
            ]);
          return prev.filter((a) => a.name !== appName);
        });
        setCaskUpToDate((prev) => prev.filter((a) => a.name !== appName));
        showToast({ ok: true, msg: `${appName} mis à jour` });
      } else {
        showToast({ ok: false, msg: `Échec : ${appName}`, details: lines.join("\n") });
      }
    });
    invoke("update_brew_app", { name: appName, downloadUrl, brewManaged }).catch((err) => {
      setBrewBusy(null);
      u1();
      u2();
      showToast({ ok: false, msg: `Échec : ${appName}`, details: String(err) });
    });
  };

  const handleMasUpdate = async (app: AppStoreUpdate) => {
    setMasBusy(app.name);
    const lines: string[] = [];
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<string>("mas-output", (e) => lines.push(e.payload));
    const u2 = await listen<boolean>("mas-done", (e) => {
      setMasBusy(null);
      u1();
      u2();
      if (e.payload) {
        setMasApps((prev) => {
          const a = prev.find((x) => x.name === app.name);
          if (a) setMasUpToDate((u) => [...u, { name: a.name, current_version: a.latest_version }]);
          return prev.filter((x) => x.name !== app.name);
        });
        showToast({ ok: true, msg: `${app.name} mis à jour` });
      } else {
        showToast({ ok: false, msg: `Échec : ${app.name}`, details: lines.join("\n") });
      }
    });
    invoke("update_mas_app", { trackId: app.track_id, name: app.name }).catch((err) => {
      const details = String(err).includes("mas_not_installed")
        ? t.update_mas_missing
        : String(err);
      setMasBusy(null);
      u1();
      u2();
      showToast({ ok: false, msg: `Échec : ${app.name}`, details });
    });
  };

  const handleSparkleUpdate = async (update: SparkleUpdate) => {
    setSparkleBusy(update.name);
    const lines: string[] = [];
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<string>("sparkle-output", (e) => lines.push(e.payload));
    const u2 = await listen<boolean>("sparkle-done", (e) => {
      setSparkleBusy(null);
      u1();
      u2();
      if (e.payload) {
        setSparkleApps((prev) => {
          const a = prev.find((x) => x.name === update.name);
          if (a)
            setSparkleUpToDate((u) => [...u, { name: a.name, current_version: a.latest_version }]);
          return prev.filter((x) => x.name !== update.name);
        });
        showToast({ ok: true, msg: `${update.name} mis à jour` });
      } else {
        showToast({ ok: false, msg: `Échec : ${update.name}`, details: lines.join("\n") });
      }
    });
    invoke("install_sparkle_update", {
      name: update.name,
      downloadUrl: update.download_url,
      appPath: update.path,
    }).catch((err) => {
      setSparkleBusy(null);
      u1();
      u2();
      showToast({ ok: false, msg: `Échec : ${update.name}`, details: String(err) });
    });
  };

  // ── Filtered lists ────────────────────────────────────────────────────────────

  const uninstalled = new Set([...uninstalledNames].map((n) => n.toLowerCase()));
  const sparkleNames = new Set(
    [...sparkleApps, ...sparkleUpToDate].map((a) => a.name.toLowerCase())
  );
  const brewNames = new Set([...brewApps, ...brewUpToDate].map((a) => a.name.toLowerCase()));

  const brewFiltered = brewApps.filter(
    (a) => a.brew_managed && !uninstalled.has(a.name.toLowerCase())
  );
  const brewUtdFiltered = brewUpToDate.filter((a) => !uninstalled.has(a.name.toLowerCase()));
  const caskFiltered = brewApps.filter(
    (a) => !a.brew_managed && !uninstalled.has(a.name.toLowerCase())
  );
  const caskUtdFiltered = caskUpToDate.filter((a) => !uninstalled.has(a.name.toLowerCase()));
  const sparkleFiltered = sparkleApps.filter((a) => !uninstalled.has(a.name.toLowerCase()));
  const sparkleUtdFiltered = sparkleUpToDate.filter((a) => !uninstalled.has(a.name.toLowerCase()));
  const masFiltered = masApps.filter(
    (a) =>
      !sparkleNames.has(a.name.toLowerCase()) &&
      !brewNames.has(a.name.toLowerCase()) &&
      !uninstalled.has(a.name.toLowerCase())
  );
  const masUtdFiltered = masUpToDate.filter(
    (a) =>
      !sparkleNames.has(a.name.toLowerCase()) &&
      !brewNames.has(a.name.toLowerCase()) &&
      !uninstalled.has(a.name.toLowerCase())
  );

  const appSectionCount = sparkleFiltered.length + caskFiltered.length;
  const totalPending =
    brewFiltered.length + caskFiltered.length + sparkleFiltered.length + masFiltered.length;
  const anyBusy = brewBusy !== null || sparkleBusy !== null || masBusy !== null;

  // Page de chargement initiale (brew + MAS sont les sources principales)
  if (brewLoading || masLoading) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6">
        <div className="flex flex-col gap-3 w-56">
          <p
            className="text-xs font-semibold uppercase tracking-widest text-center mb-2"
            style={{ color: "var(--text-3)" }}
          >
            Chargement
          </p>
          {loadingItems.map((item) => (
            <div key={item.id} className="flex items-center gap-3">
              {item.done ? (
                <CheckCircle2 size={13} style={{ color: "var(--success)", flexShrink: 0 }} />
              ) : (
                <Loader2
                  size={13}
                  className="animate-spin"
                  style={{ color: "var(--accent)", flexShrink: 0 }}
                />
              )}
              <span
                className="text-[11px]"
                style={{ color: item.done ? "var(--text-3)" : "var(--text-1)" }}
              >
                {item.label}
              </span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full gap-2.5 relative">
      {/* Global update all */}
      {totalPending > 1 && (
        <div className="flex justify-end">
          <button
            onClick={() => {
              brewFiltered.forEach((a) => handleBrewUpdate(a.name, true, a.download_url));
              caskFiltered.forEach((a) => handleBrewUpdate(a.name, false, a.download_url));
              sparkleFiltered.forEach((a) => handleSparkleUpdate(a));
            }}
            disabled={anyBusy}
            className="flex items-center gap-1 py-1 px-2.5 rounded-full text-[10px] font-medium disabled:opacity-40"
            style={{ background: "var(--danger-dim)", color: "var(--danger)" }}
          >
            <Download size={10} /> {t.update_update_all} ({totalPending})
          </button>
        </div>
      )}

      <div className="flex-1 overflow-y-auto flex flex-col gap-4 pr-0.5">
        {/* ── Homebrew ─────────────────────────────────── */}
        {(brewLoading || brewCheckedCount > 0 || brewFiltered.length > 0) && (
          <div className="flex flex-col gap-1.5">
            <SectionHeader
              label="Homebrew"
              count={brewLoading ? undefined : brewFiltered.length}
              loading={brewLoading}
              onUpdateAll={
                brewFiltered.length > 0
                  ? () =>
                      brewFiltered.forEach((a) => handleBrewUpdate(a.name, true, a.download_url))
                  : undefined
              }
              pendingCount={brewFiltered.length}
            />
            {!brewLoading && (
              <>
                {brewFiltered.map((app) => (
                  <UpdateRow
                    key={app.name}
                    name={app.name}
                    from={app.installed_version}
                    to={app.current_version}
                    onUpdate={() => handleBrewUpdate(app.name, true, app.download_url)}
                    busy={brewBusy === app.name}
                    icons={icons}
                  />
                ))}
                {brewUtdFiltered.map((app) => (
                  <UpdateRow
                    key={app.name + "-utd"}
                    name={app.name}
                    from={app.current_version}
                    upToDate
                    icons={icons}
                  />
                ))}
              </>
            )}
          </div>
        )}

        {/* ── App Store ─────────────────────────────────── */}
        {(masLoading || masCheckedCount > 0) && (
          <div className="flex flex-col gap-1.5">
            <SectionHeader
              label="App Store"
              count={masLoading ? undefined : masFiltered.length}
              loading={masLoading}
              onUpdateAll={
                masFiltered.length > 0
                  ? () => masFiltered.forEach((a) => handleMasUpdate(a))
                  : undefined
              }
              pendingCount={masFiltered.length}
            />
            {!masLoading && (
              <>
                {masFiltered.map((app) => (
                  <UpdateRow
                    key={app.name}
                    name={app.name}
                    from={app.installed_version}
                    to={app.latest_version}
                    notes={app.release_notes || undefined}
                    onUpdate={
                      app.from_store
                        ? () => handleMasUpdate(app)
                        : async () => {
                            await invoke("open_app_store_url", { url: app.store_url });
                          }
                    }
                    busy={masBusy === app.name}
                    icons={icons}
                    actionLabel={app.from_store ? t.uninstall_update : t.uninstall_open}
                  />
                ))}
                {masUtdFiltered.map((app) => (
                  <UpdateRow
                    key={app.name + "-utd"}
                    name={app.name}
                    from={app.current_version}
                    upToDate
                    icons={icons}
                  />
                ))}
              </>
            )}
          </div>
        )}

        {/* ── Applications (Sparkle + Cask non-brew) ────── */}
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <SectionHeader
              label="Applications"
              count={sparkleChecked && !sparkleLoading ? appSectionCount : undefined}
              loading={sparkleLoading || brewLoading}
              onUpdateAll={
                appSectionCount > 0
                  ? () => {
                      caskFiltered.forEach((a) => handleBrewUpdate(a.name, false, a.download_url));
                      sparkleFiltered.forEach((a) => handleSparkleUpdate(a));
                    }
                  : undefined
              }
              pendingCount={appSectionCount}
            />
            {!sparkleLoading && (
              <button
                onClick={checkSparkle}
                className="text-[10px]"
                style={{ color: "var(--text-3)" }}
              >
                {t.uninstall_recheck}
              </button>
            )}
          </div>
          {sparkleLoading ? (
            <p className="text-[10px] px-1" style={{ color: "var(--text-3)" }}>
              {t.uninstall_querying}
            </p>
          ) : (
            <>
              {caskFiltered.map((app) => (
                <UpdateRow
                  key={app.name}
                  name={app.name}
                  from={app.installed_version}
                  to={app.current_version}
                  onUpdate={() => handleBrewUpdate(app.name, false, app.download_url)}
                  busy={brewBusy === app.name}
                  icons={icons}
                />
              ))}
              {sparkleFiltered.map((app) => (
                <UpdateRow
                  key={app.name}
                  name={app.name}
                  from={app.current_version}
                  to={app.latest_version}
                  notes={app.release_notes || undefined}
                  onUpdate={() => handleSparkleUpdate(app)}
                  busy={sparkleBusy === app.name}
                  icons={icons}
                />
              ))}
              {caskUtdFiltered.map((app) => (
                <UpdateRow
                  key={app.name + "-cutd"}
                  name={app.name}
                  from={app.current_version}
                  upToDate
                  icons={icons}
                />
              ))}
              {sparkleUtdFiltered.map((app) => (
                <UpdateRow
                  key={app.name + "-utd"}
                  name={app.name}
                  from={app.current_version}
                  upToDate
                  icons={icons}
                />
              ))}
            </>
          )}
        </div>
      </div>

      {/* Toast + panneau de détail erreur */}
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
            {toast.ok ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
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

// ── Tab: Explorer (Homebrew cask browser) ─────────────────────────────────────

interface CaskInfo {
  token: string;
  name: string;
  desc: string;
}

type ExpCategory =
  "all" | "browsers" | "dev" | "media" | "design" | "productivity" | "security" | "chat" | "utils";

const EXP_CATEGORIES: { id: ExpCategory; label: string }[] = [
  { id: "all", label: "Tout" },
  { id: "dev", label: "Développement" },
  { id: "browsers", label: "Navigateurs" },
  { id: "media", label: "Médias" },
  { id: "design", label: "Design" },
  { id: "productivity", label: "Productivité" },
  { id: "security", label: "Sécurité" },
  { id: "chat", label: "Communication" },
  { id: "utils", label: "Utilitaires" },
];

const CAT_KEYWORDS: Record<Exclude<ExpCategory, "all">, string[]> = {
  browsers: ["browser", "chromium", "firefox", "webkit", "tab management"],
  dev: [
    "editor",
    "ide",
    "developer",
    "development",
    "programming",
    "code",
    "git",
    "terminal",
    "database",
    "docker",
    "kubernetes",
    "debugger",
    "compiler",
    "runtime",
    "sdk",
    "api client",
    "http client",
  ],
  media: [
    "video",
    "audio",
    "music",
    "player",
    "streaming",
    "podcast",
    "photo",
    "image editor",
    "media",
  ],
  design: [
    "design",
    "graphic",
    "creative",
    "vector",
    "prototype",
    "wireframe",
    "3d model",
    "font",
    "color picker",
  ],
  productivity: [
    "office",
    "productivity",
    "notes",
    "note-taking",
    "task",
    "calendar",
    "email",
    "pdf",
    "spreadsheet",
    "word processor",
    "writing",
    "clipboard",
  ],
  security: [
    "vpn",
    "security",
    "password",
    "firewall",
    "antivirus",
    "privacy",
    "encryption",
    "tunnel",
  ],
  chat: [
    "chat",
    "messaging",
    "message",
    "video call",
    "conference",
    "collaboration",
    "voice",
    "teams",
  ],
  utils: [
    "utility",
    "utilities",
    "system",
    "monitor",
    "backup",
    "cleaner",
    "compress",
    "archiver",
    "launcher",
    "menu bar",
    "window management",
    "file manager",
  ],
};

function classifyCask(token: string, desc: string): Exclude<ExpCategory, "all"> {
  const text = `${token} ${desc}`.toLowerCase();
  for (const [cat, keywords] of Object.entries(CAT_KEYWORDS) as [
    Exclude<ExpCategory, "all">,
    string[],
  ][]) {
    if (keywords.some((kw) => text.includes(kw))) return cat;
  }
  return "utils";
}

// ── Data loading ──────────────────────────────────────────────────────────────

const PAGE_SIZE = 30;

async function loadTopCasks(
  limit = 100
): Promise<{ casks: CaskInfo[]; popularity: Map<string, number> }> {
  const analyticsRes = await fetch(
    "https://formulae.brew.sh/api/analytics/cask-install/homebrew-cask/90d.json"
  );
  const analytics = await analyticsRes.json();
  const formulae = analytics.formulae as Record<string, { cask: string; count: string }[]>;

  const entries = Object.entries(formulae)
    .map(([token, data]) => ({
      token,
      count: parseInt(data[0]?.count?.replace(/,/g, "") ?? "0", 10),
    }))
    .sort((a, b) => b.count - a.count);

  const popularity = new Map(entries.map((e) => [e.token, e.count] as [string, number]));
  const tokens = entries.slice(0, limit).map((e) => e.token);

  const casks: CaskInfo[] = [];
  const BATCH = 10;
  for (let i = 0; i < tokens.length; i += BATCH) {
    const batch = tokens.slice(i, i + BATCH);
    const settled = await Promise.allSettled(
      batch.map((token) =>
        fetch(`https://formulae.brew.sh/api/cask/${token}.json`)
          .then((r) => r.json())
          .then(
            (d) =>
              ({
                token: String(d.token ?? token),
                name: Array.isArray(d.name) ? String(d.name[0] ?? token) : String(d.name ?? token),
                desc: String(d.desc ?? ""),
              }) as CaskInfo
          )
      )
    );
    for (const r of settled) {
      if (r.status === "fulfilled") casks.push(r.value);
    }
  }
  return { casks, popularity };
}

function CaskCard({
  cask,
  installed,
  installing,
  onInstall,
}: {
  cask: CaskInfo;
  installed: boolean;
  installing: boolean;
  onInstall: () => void;
}) {
  return (
    <div className="card flex flex-col gap-2 p-3">
      <div className="flex items-start gap-2">
        <div
          className="w-8 h-8 rounded-lg flex items-center justify-center text-xs font-bold shrink-0 mt-0.5"
          style={{ background: "var(--accent-dim)", color: "var(--accent-text)" }}
        >
          {cask.name[0]?.toUpperCase() ?? "?"}
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-xs font-semibold truncate" style={{ color: "var(--text-1)" }}>
            {cask.name}
          </div>
          <div className="text-[9px] font-mono truncate" style={{ color: "var(--text-3)" }}>
            {cask.token}
          </div>
        </div>
      </div>
      <p
        className="text-[10px] leading-relaxed line-clamp-2 flex-1"
        style={{ color: "var(--text-2)" }}
      >
        {cask.desc || "—"}
      </p>
      <button
        onClick={onInstall}
        disabled={installed || installing}
        className="flex items-center justify-center gap-1 py-1.5 px-2 rounded-lg text-[10px] font-semibold transition-all disabled:opacity-60"
        style={{
          background: installed ? "var(--bar-track)" : "var(--accent-dim)",
          color: installed ? "var(--text-3)" : "var(--accent-text)",
        }}
      >
        {installing ? (
          <>
            <Loader2 size={9} className="animate-spin" /> Installation…
          </>
        ) : installed ? (
          <>
            <CheckCircle2 size={9} /> Installé
          </>
        ) : (
          <>
            <Download size={9} /> Installer
          </>
        )}
      </button>
    </div>
  );
}

function ExplorerTab({
  onInstallSuccess,
  installedRefreshKey,
}: {
  onInstallSuccess?: () => void;
  installedRefreshKey?: number;
}) {
  const [topCasks, setTopCasks] = useState<CaskInfo[]>([]);
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [catalogReady, setCatalogReady] = useState(false);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<ExpCategory>("all");
  const [installing, setInstalling] = useState<string | null>(null);
  const [toast, setToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const [displayCount, setDisplayCount] = useState(PAGE_SIZE);

  const fullCatalogRef = useRef<CaskInfo[]>([]);
  const popularityRef = useRef<Map<string, number>>(new Map());

  // Initial load: top 100 popular + installed list
  useEffect(() => {
    (async () => {
      const [caskResult, installedResult] = await Promise.allSettled([
        loadTopCasks(100),
        invoke<string[]>("get_installed_casks").catch(() => [] as string[]),
      ]);
      if (caskResult.status === "fulfilled") {
        setTopCasks(caskResult.value.casks);
        popularityRef.current = caskResult.value.popularity;
      }
      if (installedResult.status === "fulfilled") setInstalled(new Set(installedResult.value));
      setLoading(false);
    })();
  }, []);

  // Background: load full catalog (~7000 casks) sorted by popularity
  useEffect(() => {
    fetch("https://formulae.brew.sh/api/cask.json")
      .then((r) => r.json())
      .then((data: Record<string, unknown>[]) => {
        const pop = popularityRef.current;
        fullCatalogRef.current = data
          .map((d) => ({
            token: String(d.token ?? ""),
            name: Array.isArray(d.name)
              ? String((d.name as string[])[0] ?? "")
              : String(d.name ?? ""),
            desc: String(d.desc ?? ""),
            homepage: String(d.homepage ?? ""),
          }))
          .filter((c) => c.token)
          .sort((a, b) => {
            const pa = pop.get(a.token) ?? 0;
            const pb = pop.get(b.token) ?? 0;
            return pb !== pa ? pb - pa : a.token.localeCompare(b.token);
          });
        setCatalogReady(true);
      })
      .catch((e) => console.error("get_brew_casks:", e));
  }, []);

  useEffect(() => {
    if (!installedRefreshKey) return;
    invoke<string[]>("get_installed_casks")
      .then((list) => setInstalled(new Set(list)))
      .catch((e) => console.error("get_installed_casks:", e));
  }, [installedRefreshKey]);

  const handleInstall = async (cask: CaskInfo) => {
    flushSync(() => setInstalling(cask.token));
    try {
      await invoke("install_brew_cask", { token: cask.token });
      setInstalled((prev) => new Set([...prev, cask.token]));
      setToast({ ok: true, msg: `${cask.name} installé` });
      onInstallSuccess?.();
    } catch {
      setToast({ ok: false, msg: `Échec : ${cask.name}` });
    } finally {
      setInstalling(null);
      setTimeout(() => setToast(null), 3500);
    }
  };

  const handleQuery = (q: string) => {
    setQuery(q);
    setDisplayCount(PAGE_SIZE);
  };
  const handleCategory = (c: ExpCategory) => {
    setCategory(c);
    setDisplayCount(PAGE_SIZE);
  };

  const filtered = useMemo(() => {
    const source = catalogReady ? fullCatalogRef.current : topCasks;
    const q = query.toLowerCase();
    return source.filter((c) => {
      const matchQuery =
        !q ||
        c.token.includes(q) ||
        c.name.toLowerCase().includes(q) ||
        c.desc.toLowerCase().includes(q);
      const matchCat = category === "all" || classifyCask(c.token, c.desc) === category;
      return matchQuery && matchCat;
    });
  }, [topCasks, query, category, catalogReady]);

  const displayed = filtered.slice(0, displayCount);
  const hasMore = displayCount < filtered.length;

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3">
        <Loader2 size={22} className="animate-spin" style={{ color: "var(--accent)" }} />
        <span className="text-sm" style={{ color: "var(--text-3)" }}>
          Chargement du catalogue Homebrew…
        </span>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full gap-2 relative">
      {/* Search */}
      <div className="card flex items-center gap-2 px-2.5 py-1.5">
        <Search size={12} style={{ color: "var(--text-3)", flexShrink: 0 }} />
        <input
          value={query}
          onChange={(e) => handleQuery(e.target.value)}
          placeholder={catalogReady ? "Rechercher dans tout Homebrew…" : "Rechercher…"}
          className="flex-1 bg-transparent text-xs outline-none"
          style={{ color: "var(--text-1)" }}
        />
        {!catalogReady && (
          <Loader2 size={9} className="animate-spin shrink-0" style={{ color: "var(--text-3)" }} />
        )}
        {query && (
          <button onClick={() => handleQuery("")} style={{ color: "var(--text-3)" }}>
            <X size={10} />
          </button>
        )}
      </div>

      {/* Categories + count */}
      <div className="flex items-center gap-1 flex-wrap">
        {EXP_CATEGORIES.map((cat) => (
          <button
            key={cat.id}
            onClick={() => handleCategory(cat.id)}
            className="px-2.5 py-1 rounded-full text-[10px] font-medium transition-all"
            style={{
              background: category === cat.id ? "var(--accent-dim)" : "var(--bar-track)",
              color: category === cat.id ? "var(--accent-text)" : "var(--text-3)",
            }}
          >
            {cat.label}
          </button>
        ))}
        <span className="ml-auto text-[9px] shrink-0" style={{ color: "var(--text-3)" }}>
          {filtered.length} app{filtered.length !== 1 ? "s" : ""}
          {!catalogReady && " ·  "}
          {!catalogReady && <Loader2 size={8} className="animate-spin inline ml-0.5" />}
        </span>
      </div>

      {/* Grid + load more */}
      <div className="flex-1 overflow-y-auto pr-0.5">
        {filtered.length === 0 ? (
          <div
            className="flex items-center justify-center h-full text-xs"
            style={{ color: "var(--text-3)" }}
          >
            Aucun résultat
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <div className="grid grid-cols-3 gap-2">
              {displayed.map((cask) => (
                <CaskCard
                  key={cask.token}
                  cask={cask}
                  installed={installed.has(cask.token)}
                  installing={installing === cask.token}
                  onInstall={() => handleInstall(cask)}
                />
              ))}
            </div>
            {hasMore && (
              <button
                onClick={() => setDisplayCount((prev) => prev + PAGE_SIZE)}
                className="w-full py-2 rounded-xl text-[10px] font-medium transition-colors"
                style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
              >
                Afficher {Math.min(PAGE_SIZE, filtered.length - displayCount)} de plus
                <span style={{ color: "var(--text-3)" }}>
                  {" "}
                  · {filtered.length - displayCount} restants
                </span>
              </button>
            )}
          </div>
        )}
      </div>

      {/* Toast */}
      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.95 }}
            className="fixed bottom-4 right-4 z-50 flex items-center gap-2 px-4 py-2.5 rounded-xl text-[12px] font-medium shadow-lg"
            style={{ background: toast.ok ? "var(--success)" : "var(--danger)", color: "#fff" }}
          >
            {toast.ok ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
            {toast.msg}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ── Tab: Packages (Homebrew formulae) ─────────────────────────────────────────

function PackagesTab({ icons }: { icons: Record<string, string> }) {
  const [formulaApps, setFormulaApps] = useState<BrewOutdated[]>([]);
  const [formulaUtd, setFormulaUtd] = useState<UpToDateApp[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const processingRef = useRef(false);
  const queueRef = useRef<string[]>([]);

  const load = useCallback(() => {
    setLoading(true);
    invoke<BrewFormulaResult>("get_brew_formula_outdated")
      .then((r) => {
        setFormulaApps(r.updates);
        setFormulaUtd(r.up_to_date);
      })
      .catch((e) => console.error("get_brew_formula_outdated:", e))
      .finally(() => setLoading(false));
  }, []);
  useEffect(() => {
    load();
  }, [load]);

  const processQueue = useCallback(async () => {
    const total = queueRef.current.length;
    const succeeded: string[] = [];
    const failed: string[] = [];

    while (queueRef.current.length > 0) {
      const name = queueRef.current.shift()!;
      setBusy(name);
      const ok = await new Promise<boolean>((resolve) => {
        (async () => {
          const { listen } = await import("@tauri-apps/api/event");
          const u1 = await listen<string>("brew-formula-output", () => {});
          const u2 = await listen<number>("brew-formula-done", (e) => {
            u1();
            u2();
            resolve(e.payload === 0);
          });
          invoke("update_brew_formula", { name }).catch(() => {
            u1();
            u2();
            resolve(false);
          });
        })();
      });
      if (ok) {
        succeeded.push(name);
        setFormulaApps((prev) => {
          const a = prev.find((x) => x.name === name);
          if (a) setFormulaUtd((u) => [...u, { name: a.name, current_version: a.current_version }]);
          return prev.filter((x) => x.name !== name);
        });
      } else {
        failed.push(name);
      }
    }

    setBusy(null);
    processingRef.current = false;
    const msg =
      total === 1
        ? succeeded.length === 1
          ? `${succeeded[0]} mis à jour`
          : `Échec : ${failed[0]}`
        : failed.length > 0
          ? `${succeeded.length}/${total} mis à jour · ${failed.length} erreur(s)`
          : `${total} packages mis à jour`;
    setToast({ ok: failed.length === 0, msg });
    setTimeout(() => setToast(null), 4000);
  }, []);

  const handleUpdate = useCallback(
    (name: string) => {
      if (processingRef.current) {
        if (!queueRef.current.includes(name)) queueRef.current.push(name);
        return;
      }
      processingRef.current = true;
      queueRef.current = [name];
      processQueue();
    },
    [processQueue]
  );

  const handleUpdateAll = useCallback(() => {
    if (processingRef.current) return;
    const names = formulaApps.map((a) => a.name);
    if (names.length === 0) return;
    processingRef.current = true;
    queueRef.current = [...names];
    processQueue();
  }, [formulaApps, processQueue]);

  const pending = formulaApps.length;

  return (
    <div className="flex flex-col h-full gap-2.5 relative">
      <div className="flex items-center justify-between">
        <SectionHeader
          label="Homebrew Packages"
          count={loading ? undefined : pending}
          loading={loading}
          onUpdateAll={pending > 0 ? handleUpdateAll : undefined}
          pendingCount={pending}
        />
        <button
          onClick={load}
          disabled={loading}
          className="p-1 transition-colors disabled:opacity-30"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto flex flex-col gap-1">
        {loading ? (
          <div
            className="flex items-center justify-center py-10 gap-2"
            style={{ color: "var(--text-3)" }}
          >
            <Loader2 size={14} className="animate-spin" />
            <span className="text-xs">Vérification des packages…</span>
          </div>
        ) : formulaApps.length === 0 && formulaUtd.length === 0 ? (
          <div className="text-center py-10 text-xs" style={{ color: "var(--text-3)" }}>
            Homebrew non installé ou aucun package trouvé
          </div>
        ) : (
          <>
            {formulaApps.map((app) => (
              <UpdateRow
                key={app.name}
                name={app.name}
                from={app.installed_version}
                to={app.current_version}
                onUpdate={() => handleUpdate(app.name)}
                busy={busy === app.name}
                icons={icons}
              />
            ))}
            {formulaUtd.map((app) => (
              <UpdateRow
                key={app.name + "-utd"}
                name={app.name}
                from={app.current_version}
                upToDate
                icons={icons}
              />
            ))}
          </>
        )}
      </div>

      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.95 }}
            transition={{ duration: 0.2 }}
            className="fixed bottom-4 right-4 z-50 flex items-center gap-2.5 px-4 py-2.5 rounded-xl shadow-lg text-sm font-medium"
            style={{ background: toast.ok ? "var(--success)" : "var(--danger)", color: "#fff" }}
          >
            {toast.ok ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
            {toast.msg}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────

type TabId = "uninstall" | "updates" | "packages" | "explorer";

export default function Apps() {
  const { t } = useT();
  const [tab, setTab] = useState<TabId>("uninstall");
  const [appIcons, setAppIcons] = useState<Record<string, string>>({});
  const [uninstalledNames, setUninstalled] = useState<Set<string>>(new Set());
  const [appsRefreshKey, setAppsRefreshKey] = useState(0);
  // Montage paresseux : chaque onglet n'est monté qu'au premier clic
  const [updatesMounted, setUpdatesMounted] = useState(false);
  const [packagesMounted, setPackagesMounted] = useState(false);
  const [explorerMounted, setExplorerMounted] = useState(false);

  useEffect(() => {
    invoke<Record<string, string>>("get_all_app_icons")
      .then(setAppIcons)
      .catch((e) => console.error("get_all_app_icons:", e));
  }, []);

  useEffect(() => {
    if (tab === "updates") setUpdatesMounted(true);
    if (tab === "packages") setPackagesMounted(true);
    if (tab === "explorer") setExplorerMounted(true);
  }, [tab]);

  const TABS: { id: TabId; label: string; icon: React.ElementType }[] = [
    { id: "uninstall", label: t.uninstall_tab_uninstall, icon: Trash2 },
    { id: "updates", label: t.uninstall_tab_updates, icon: Download },
    { id: "packages", label: "Packages", icon: Package },
    { id: "explorer", label: "Explorer", icon: Search },
  ];

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-3">
      {/* Subtab bar */}
      <div className="flex items-center gap-0.5 pt-1">
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all"
            style={{
              color: tab === id ? "var(--text-1)" : "var(--text-3)",
              background: tab === id ? "var(--bar-track)" : "transparent",
              fontWeight: tab === id ? 600 : 400,
            }}
          >
            <Icon size={11} />
            {label}
          </button>
        ))}
      </div>

      {/* Content — montage paresseux : chaque onglet monte au premier clic */}
      <div className="flex-1 min-h-0">
        <div
          style={{
            display: tab === "uninstall" ? "flex" : "none",
            flexDirection: "column",
            height: "100%",
          }}
        >
          <UninstallTab
            icons={appIcons}
            onUninstalled={(name) => {
              setUninstalled((prev) => new Set([...prev, name]));
              setAppsRefreshKey((k) => k + 1);
            }}
            refreshKey={appsRefreshKey}
          />
        </div>
        {updatesMounted && (
          <div
            style={{
              display: tab === "updates" ? "flex" : "none",
              flexDirection: "column",
              height: "100%",
            }}
          >
            <UpdatesTab icons={appIcons} uninstalledNames={uninstalledNames} />
          </div>
        )}
        {packagesMounted && (
          <div
            style={{
              display: tab === "packages" ? "flex" : "none",
              flexDirection: "column",
              height: "100%",
            }}
          >
            <PackagesTab icons={appIcons} />
          </div>
        )}
        {explorerMounted && (
          <div
            style={{
              display: tab === "explorer" ? "flex" : "none",
              flexDirection: "column",
              height: "100%",
            }}
          >
            <ExplorerTab
              onInstallSuccess={() => setAppsRefreshKey((k) => k + 1)}
              installedRefreshKey={appsRefreshKey}
            />
          </div>
        )}
      </div>
    </div>
  );
}
