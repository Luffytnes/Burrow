import { useState, useEffect, useCallback } from "react";
import { motion } from "framer-motion";
import {
  HardDrive,
  Trash2,
  Loader2,
  RefreshCw,
  Package,
  Layers,
  FileText,
  Shield,
  AlertTriangle,
  AlertCircle,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";

interface DiskCategory {
  id: string;
  name: string;
  path: string;
  size_bytes: number;
}
interface DevCache {
  id: string;
  name: string;
  path: string;
  size_bytes: number;
  risk: number;
  days_since_use: number;
}
interface ProjectArtifact {
  project_name: string;
  project_path: string;
  artifact_type: string;
  artifact_path: string;
  size_bytes: number;
}
interface LargeFile {
  name: string;
  path: string;
  size_bytes: number;
  days_old: number;
}

function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} Go`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} Mo`;
  if (b >= 1e3) return `${Math.round(b / 1e3)} Ko`;
  return `${b} o`;
}

const CAT_COLORS: Record<string, string> = {
  applications: "var(--info)",
  documents: "var(--warning)",
  downloads: "var(--cyan)",
  desktop: "var(--violet)",
  developer: "var(--danger-text)",
  movies: "var(--danger)",
  music: "var(--pink)",
  pictures: "var(--success)",
  trash: "var(--text-3)",
};

const RISK_COLOR = ["var(--success)", "var(--warning)", "var(--danger)"] as const;
const RISK_LABEL = ["Sûr", "Attention", "Risqué"] as const;
const RISK_ICON = [Shield, AlertTriangle, AlertCircle] as const;

function TrashBtn({ path, size, onDone }: { path: string; size: number; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [err, setErr] = useState(false);

  const handle = async () => {
    setBusy(true);
    setErr(false);
    try {
      await invoke("move_to_trash", { path });
      setDone(true);
      setTimeout(onDone, 350);
    } catch {
      setBusy(false);
      setErr(true);
      setTimeout(() => setErr(false), 2000);
    }
  };

  if (done)
    return (
      <span className="text-xs shrink-0" style={{ color: "var(--success)" }}>
        ✓
      </span>
    );
  if (err)
    return (
      <span className="text-xs shrink-0" style={{ color: "var(--danger)" }}>
        ✗
      </span>
    );
  return (
    <div className="flex items-center gap-2 shrink-0">
      <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
        {fmtBytes(size)}
      </span>
      <button
        onClick={handle}
        disabled={busy}
        className="p-1 rounded transition-opacity disabled:opacity-40"
        style={{ color: "var(--danger)" }}
        onMouseEnter={(e) => (e.currentTarget.style.opacity = "0.7")}
        onMouseLeave={(e) => (e.currentTarget.style.opacity = "1")}
      >
        {busy ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
      </button>
    </div>
  );
}

// ── Tab 1 : Vue d'ensemble ────────────────────────────────────────────────────

function OverviewTab() {
  const [categories, setCategories] = useState<DiskCategory[] | null>(null);
  const [diskInfo, setDiskInfo] = useState<{ used: number; total: number } | null>(null);
  const [trashSize, setTrashSize] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [cats, metrics] = await Promise.all([
        invoke<DiskCategory[]>("get_disk_categories"),
        invoke<{ disk_used: number; disk_total: number }>("get_quick_metrics"),
      ]);
      setCategories(cats);
      setDiskInfo({ total: metrics.disk_total, used: metrics.disk_used });
      const trash = cats.find((c) => c.id === "trash");
      if (trash) setTrashSize(trash.size_bytes);
    } catch (e) {
      console.error("load disk overview:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const usedPct = diskInfo ? (diskInfo.used / diskInfo.total) * 100 : 0;
  const nonTrash = categories?.filter((c) => c.id !== "trash") ?? [];
  const totalCatB = nonTrash.reduce((s, c) => s + c.size_bytes, 0);
  const barColor = usedPct > 85 ? "var(--danger)" : usedPct > 70 ? "var(--warning)" : "var(--info)";

  return (
    <div className="flex flex-col gap-3">
      {/* Disk bar */}
      <div className="card p-4">
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
            Macintosh HD
          </span>
          <div className="flex items-center gap-2">
            <span className="text-xs" style={{ color: "var(--text-3)" }}>
              {diskInfo ? `${fmtBytes(diskInfo.used)} / ${fmtBytes(diskInfo.total)}` : "…"}
            </span>
            <button
              onClick={load}
              disabled={loading}
              className="p-1 transition-opacity disabled:opacity-40"
              style={{ color: "var(--text-3)" }}
            >
              <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
            </button>
          </div>
        </div>
        <div
          className="h-3 rounded-full overflow-hidden"
          style={{ background: "var(--bar-track)" }}
        >
          <motion.div
            initial={{ width: 0 }}
            animate={{ width: `${usedPct}%` }}
            transition={{ duration: 0.6, ease: "easeOut" }}
            className="h-full rounded-full"
            style={{ background: barColor }}
          />
        </div>
        <div className="flex justify-between mt-1.5 text-[11px]" style={{ color: "var(--text-3)" }}>
          <span>{diskInfo ? fmtBytes(diskInfo.total - diskInfo.used) + " libre" : ""}</span>
          <span>{diskInfo ? Math.round(usedPct) + "% utilisé" : ""}</span>
        </div>
      </div>

      {/* Categories */}
      {loading ? (
        <div
          className="flex items-center justify-center py-10 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Analyse en cours…</span>
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {nonTrash.map((cat, i) => {
            const color = CAT_COLORS[cat.id] ?? "var(--text-3)";
            const pct = totalCatB > 0 ? (cat.size_bytes / totalCatB) * 100 : 0;
            return (
              <motion.div
                key={cat.id}
                initial={{ opacity: 0, x: -8 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: i * 0.04 }}
                className="card px-4 py-2.5"
              >
                <div className="flex items-center justify-between mb-1.5">
                  <span className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {cat.name}
                  </span>
                  <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
                    {fmtBytes(cat.size_bytes)}
                  </span>
                </div>
                <div className="h-1 rounded-full" style={{ background: "var(--bar-track)" }}>
                  <motion.div
                    initial={{ width: 0 }}
                    animate={{ width: `${Math.min(pct, 100)}%` }}
                    transition={{ delay: 0.1 + i * 0.04, duration: 0.5 }}
                    className="h-full rounded-full"
                    style={{ background: color }}
                  />
                </div>
              </motion.div>
            );
          })}
        </div>
      )}

      {/* Trash */}
      <div className="card px-4 py-3 flex items-center gap-3">
        <Trash2 size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
        <span className="flex-1 text-sm" style={{ color: "var(--text-1)" }}>
          Corbeille
        </span>
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {trashSize != null ? fmtBytes(trashSize) : "—"}
        </span>
        <span className="text-[10px]" style={{ color: "var(--success)" }}>
          Restauration possible via Finder
        </span>
      </div>
    </div>
  );
}

// ── Tab 2 : Caches Dev ────────────────────────────────────────────────────────

function DevCachesTab() {
  const [caches, setCaches] = useState<DevCache[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [cleanAll, setCleanAll] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setCaches(await invoke<DevCache[]>("get_dev_caches"));
    } catch (e) {
      console.error("get_dev_caches:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const safeCaches = caches?.filter((c) => c.risk === 0) ?? [];
  const totalSafe = safeCaches.reduce((s, c) => s + c.size_bytes, 0);

  const handleCleanAll = async () => {
    setCleanAll(true);
    for (const c of safeCaches) {
      try {
        await invoke("move_to_trash", { path: c.path });
      } catch (e) {
        console.error("move_to_trash:", e);
      }
    }
    await load();
    setCleanAll(false);
  };

  return (
    <div className="flex flex-col gap-3">
      {/* Action bar */}
      <div className="flex items-center justify-between">
        <span className="text-xs" style={{ color: "var(--text-3)" }}>
          {caches
            ? `${caches.length} cache${caches.length > 1 ? "s" : ""} trouvé${caches.length > 1 ? "s" : ""}`
            : ""}
        </span>
        <div className="flex items-center gap-2">
          <button
            onClick={load}
            disabled={loading}
            className="p-1.5 transition-opacity disabled:opacity-40"
            style={{ color: "var(--text-3)" }}
          >
            <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
          </button>
          {!loading && safeCaches.length > 0 && (
            <button
              onClick={handleCleanAll}
              disabled={cleanAll}
              className="text-xs px-2.5 py-1 rounded-lg transition-opacity disabled:opacity-40 flex items-center gap-1.5"
              style={{
                background: "var(--success-dim)",
                color: "var(--success)",
                border: "1px solid var(--success-soft)",
              }}
            >
              {cleanAll ? <Loader2 size={11} className="animate-spin" /> : <Trash2 size={11} />}
              Nettoyer les caches sûrs ({fmtBytes(totalSafe)})
            </button>
          )}
        </div>
      </div>

      {loading ? (
        <div
          className="flex items-center justify-center py-10 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Analyse des caches…</span>
        </div>
      ) : caches?.length === 0 ? (
        <div className="text-center py-10 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun cache trouvé
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {caches?.map((cache, i) => {
            const RiskIcon = RISK_ICON[cache.risk];
            return (
              <motion.div
                key={cache.id}
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: Math.min(i * 0.03, 0.4) }}
                className="card px-4 py-3 flex items-center gap-3"
              >
                <RiskIcon size={14} style={{ color: RISK_COLOR[cache.risk], flexShrink: 0 }} />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span
                      className="text-sm font-medium truncate"
                      style={{ color: "var(--text-1)" }}
                    >
                      {cache.name}
                    </span>
                    <span
                      className="text-[10px] px-1.5 py-0.5 rounded-full shrink-0"
                      style={{
                        background: RISK_COLOR[cache.risk] + "18",
                        color: RISK_COLOR[cache.risk],
                      }}
                    >
                      {RISK_LABEL[cache.risk]}
                    </span>
                  </div>
                  {cache.days_since_use >= 0 && (
                    <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
                      il y a {cache.days_since_use} jour{cache.days_since_use > 1 ? "s" : ""}
                    </span>
                  )}
                </div>
                <TrashBtn
                  path={cache.path}
                  size={cache.size_bytes}
                  onDone={() => setCaches((p) => p?.filter((c) => c.id !== cache.id) ?? null)}
                />
              </motion.div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── Tab 3 : Projets ───────────────────────────────────────────────────────────

function ProjectsTab() {
  const [artifacts, setArtifacts] = useState<ProjectArtifact[] | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setArtifacts(await invoke<ProjectArtifact[]>("get_project_artifacts"));
    } catch (e) {
      console.error("get_project_artifacts:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const total = artifacts?.reduce((s, a) => s + a.size_bytes, 0) ?? 0;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-xs" style={{ color: "var(--text-3)" }}>
          {artifacts
            ? `${artifacts.length} artefact${artifacts.length > 1 ? "s" : ""} · ${fmtBytes(total)}`
            : ""}
        </span>
        <button
          onClick={load}
          disabled={loading}
          className="p-1.5 transition-opacity disabled:opacity-40"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {loading ? (
        <div
          className="flex items-center justify-center py-10 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Analyse des projets…</span>
        </div>
      ) : artifacts?.length === 0 ? (
        <div className="text-center py-10 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun artefact de projet trouvé
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {artifacts?.map((a, i) => (
            <motion.div
              key={a.artifact_path}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: Math.min(i * 0.025, 0.35) }}
              className="card px-4 py-3 flex items-center gap-3"
            >
              <Package size={14} style={{ color: "var(--accent)", flexShrink: 0 }} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium truncate" style={{ color: "var(--text-1)" }}>
                    {a.project_name}
                  </span>
                  <span
                    className="text-[10px] px-1.5 py-0.5 rounded-full shrink-0"
                    style={{
                      background: "var(--bg)",
                      color: "var(--text-3)",
                      border: "1px solid var(--border)",
                    }}
                  >
                    {a.artifact_type}
                  </span>
                </div>
                <span className="text-[11px] truncate block" style={{ color: "var(--text-3)" }}>
                  {a.project_path.replace(/^\/Users\/[^/]+/, "~")}
                </span>
              </div>
              <TrashBtn
                path={a.artifact_path}
                size={a.size_bytes}
                onDone={() =>
                  setArtifacts((p) => p?.filter((x) => x.artifact_path !== a.artifact_path) ?? null)
                }
              />
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Tab 4 : Grands fichiers ───────────────────────────────────────────────────

function LargeFilesTab() {
  const [files, setFiles] = useState<LargeFile[] | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setFiles(await invoke<LargeFile[]>("get_large_files"));
    } catch (e) {
      console.error("get_large_files:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const total = files?.reduce((s, f) => s + f.size_bytes, 0) ?? 0;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-xs" style={{ color: "var(--text-3)" }}>
          {files
            ? `${files.length} fichier${files.length > 1 ? "s" : ""} >100 Mo · ${fmtBytes(total)}`
            : ""}
        </span>
        <button
          onClick={load}
          disabled={loading}
          className="p-1.5 transition-opacity disabled:opacity-40"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {loading ? (
        <div
          className="flex items-center justify-center py-10 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Recherche des grands fichiers…</span>
        </div>
      ) : files?.length === 0 ? (
        <div className="text-center py-10 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun fichier &gt;100 Mo trouvé
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {files?.map((f, i) => (
            <motion.div
              key={f.path}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: Math.min(i * 0.025, 0.35) }}
              className="card px-4 py-3 flex items-center gap-3"
            >
              <FileText size={14} style={{ color: "var(--text-3)", flexShrink: 0 }} />
              <div className="flex-1 min-w-0">
                <span
                  className="text-sm font-medium truncate block"
                  style={{ color: "var(--text-1)" }}
                >
                  {f.name}
                </span>
                <span className="text-[11px] truncate block" style={{ color: "var(--text-3)" }}>
                  {f.path.replace(/^\/Users\/[^/]+/, "~")} · il y a {f.days_old}j
                </span>
              </div>
              <TrashBtn
                path={f.path}
                size={f.size_bytes}
                onDone={() => setFiles((p) => p?.filter((x) => x.path !== f.path) ?? null)}
              />
            </motion.div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

const TABS = [
  { label: "Vue d'ensemble", Icon: HardDrive },
  { label: "Caches Dev", Icon: Layers },
  { label: "Projets", Icon: Package },
  { label: "Grands fichiers", Icon: FileText },
] as const;

export default function Analyze() {
  const [tab, setTab] = useState(0);
  const [mounted, setMounted] = useState<Set<number>>(new Set([0]));

  const switchTab = (idx: number) => {
    setTab(idx);
    setMounted((prev) => new Set([...prev, idx]));
  };

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-4">
      {/* Header */}
      <div className="flex items-center gap-3 pt-1">
        <div
          className="w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <HardDrive size={16} style={{ color: "var(--text-3)" }} />
        </div>
        <div>
          <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
            Stockage
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            Analyse et nettoyage
          </p>
        </div>
      </div>

      {/* Tab bar */}
      <div
        className="flex gap-1 p-1 rounded-xl"
        style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
      >
        {TABS.map(({ label, Icon }, idx) => (
          <button
            key={idx}
            onClick={() => switchTab(idx)}
            className="flex-1 flex items-center justify-center gap-1.5 py-1.5 px-1 rounded-lg text-[11px] font-medium transition-all"
            style={{
              background: tab === idx ? "var(--bg)" : "transparent",
              color: tab === idx ? "var(--text-1)" : "var(--text-3)",
              boxShadow: tab === idx ? "0 1px 3px rgba(0,0,0,0.1)" : "none",
            }}
          >
            <Icon size={13} />
            <span>{label}</span>
          </button>
        ))}
      </div>

      {/* Tab content – stays mounted once visited */}
      <div className="flex-1 overflow-y-auto min-h-0">
        <div style={{ display: tab === 0 ? "block" : "none" }}>
          {mounted.has(0) && <OverviewTab />}
        </div>
        <div style={{ display: tab === 1 ? "block" : "none" }}>
          {mounted.has(1) && <DevCachesTab />}
        </div>
        <div style={{ display: tab === 2 ? "block" : "none" }}>
          {mounted.has(2) && <ProjectsTab />}
        </div>
        <div style={{ display: tab === 3 ? "block" : "none" }}>
          {mounted.has(3) && <LargeFilesTab />}
        </div>
      </div>
    </div>
  );
}
