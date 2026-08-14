import { useState, useEffect, useCallback, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Trash2,
  Database,
  FileText,
  AlertCircle,
  Package,
  Loader2,
  CheckCircle2,
  Circle,
  Check,
  Globe,
  Smartphone,
  ChevronDown,
  ChevronRight,
  RefreshCw,
  Shield,
  AlertTriangle,
  HardDrive,
  Layers,
  Bot,
  Code2,
  Hammer,
  Terminal,
  Zap,
  Gamepad2,
  FlaskConical,
  Gem,
  GitBranch,
  Cloud,
  FolderOpen,
  Copy,
  ScanSearch,
  Eye,
  Cpu,
  Users,
  UserX,
  Lock,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useMo } from "../hooks/useMo";
import { useT } from "../i18n/useT";

// ── Types ─────────────────────────────────────────────────────────────────────

interface CleanCategorySize {
  id: string;
  size_mb: number;
}
interface InstallerFile {
  name: string;
  path: string;
  size_bytes: number;
  source: string;
}
interface DevCache {
  id: string;
  name: string;
  description: string;
  path: string;
  size_bytes: number;
  risk: number;
  days_since_use: number;
}
interface DerivedDataProject {
  name: string;
  path: string;
  workspace_path: string;
  size_bytes: number;
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
interface DiskMetrics {
  disk_used: number;
  disk_total: number;
  disk_used_percent: number;
}
interface DuplicateGroup {
  hash: string;
  size_bytes: number;
  paths: string[];
  wasted_bytes: number;
}
interface UniversalBinaryEntry {
  name: string;
  path: string;
  total_size_bytes: number;
  reclaimable_bytes: number;
  binary_count: number;
  thinning_unsafe: boolean;
  thinning_warning: string;
}
interface AiCacheItem {
  id: string;
  label: string;
  path: string;
  size_bytes: number;
}
interface DevCacheItem {
  id: string;
  label: string;
  path: string;
  size_bytes: number;
}
interface TmSnapshot {
  date: string;
  size_bytes: number;
}
interface SimRuntime {
  identifier: string;
  version: string;
  build: string;
  platform: string;
  size_bytes: number;
  deletable: boolean;
  last_used: string;
}
interface PurgeableInfo {
  purgeable_bytes: number;
  free_bytes: number;
  total_bytes: number;
}
interface LoginItemEntry {
  name: string;
  plist_path: string;
  program: string;
  is_broken: boolean;
  is_suspicious: boolean;
  suspicious_reason: string;
  is_system: boolean;
  can_delete: boolean;
}
interface DeletedUserEntry {
  username: string;
  home_path: string;
  size_bytes: number;
}
interface PrivacyItem {
  id: string;
  label: string;
  path: string;
  size_bytes: number;
}

// ── Session cache (persists across navigations within the same app session) ───

type _ScanCache = {
  caches?: DevCache[] | null;
  cachesLoaded?: boolean;
  mountedTabs?: number[];
  currentTab?: number;
  cleanSizes?: Record<string, number>;
  cleanInstallers?: InstallerFile[];
  cleanSizesLoaded?: boolean;
  artifacts?: ProjectArtifact[] | null;
  derivedProjects?: DerivedDataProject[] | null;
  largeFiles?: LargeFile[] | null;
  privacyItems?: PrivacyItem[] | null;
  binaries?: UniversalBinaryEntry[] | null;
  loginItems?: LoginItemEntry[] | null;
  deletedUsers?: DeletedUserEntry[] | null;
  aiCaches?: AiCacheItem[] | null;
  purgeable?: PurgeableInfo | null;
  snapshots?: TmSnapshot[] | null;
  sims?: SimRuntime[] | null;
  sysDevCaches?: DevCacheItem[] | null;
};
const _cache: _ScanCache = {};

// ── Constants ─────────────────────────────────────────────────────────────────

const CLEAN_CATS = [
  { id: "user_cache", label: "Caches utilisateur", desc: "~/Library/Caches", icon: Database },
  { id: "system_logs", label: "Logs système", desc: "~/Library/Logs", icon: FileText },
  {
    id: "crash_reports",
    label: "Rapports de crash",
    desc: "~/Library/Logs/DiagnosticReports",
    icon: AlertCircle,
  },
  {
    id: "browser_cache",
    label: "Caches navigateurs",
    desc: "Safari, Chrome, Firefox, Brave",
    icon: Globe,
  },
  {
    id: "ios_backups",
    label: "Sauvegardes iOS",
    desc: "~/Library/Application Support/MobileSync",
    icon: Smartphone,
  },
] as const;

const RISK_COLOR = ["var(--success)", "var(--warning)", "var(--danger)"] as const;
const RISK_LABEL = ["Sûr", "Attention", "Risqué"] as const;
const RISK_ICON = [Shield, AlertTriangle, AlertCircle] as const;

const TABS = [
  { label: "Aperçu", icon: HardDrive },
  { label: "Nettoyer", icon: Trash2 },
  { label: "Dev", icon: Layers },
  { label: "Fichiers", icon: FileText },
  { label: "Doublons", icon: Copy },
  { label: "Privé", icon: Eye },
  { label: "Binaires", icon: Cpu },
  { label: "Login", icon: Lock },
  { label: "AI", icon: Bot },
  { label: "Système", icon: Database },
] as const;

interface CacheGroup {
  id: string;
  label: string;
  icon: React.ElementType;
  ids: string[];
}

const CACHE_GROUPS: CacheGroup[] = [
  {
    id: "ai",
    label: "AI Tools",
    icon: Bot,
    ids: [
      "ollama",
      "huggingface",
      "claude_desktop",
      "claude_code",
      "cursor_cache",
      "cursor_data",
      "windsurf",
      "chatgpt",
    ],
  },
  {
    id: "vscode",
    label: "VS Code",
    icon: Code2,
    ids: ["vscode_cache", "vscode_data", "vscode_logs", "vscode_gpu", "vscode_ext"],
  },
  {
    id: "xcode",
    label: "Xcode",
    icon: Hammer,
    ids: ["xcode_dd", "xcode_logs", "xcode_prev", "xcode_devsup", "xcode_arch"],
  },
  { id: "ios", label: "iOS Simulateurs", icon: Smartphone, ids: ["ios_sim_cache", "ios_sim_dev"] },
  {
    id: "version",
    label: "Gestionnaires de versions",
    icon: GitBranch,
    ids: ["nvm", "pyenv", "mise_cache", "rbenv", "rvm"],
  },
  {
    id: "js",
    label: "JavaScript / Node",
    icon: Zap,
    ids: ["npm", "yarn_v1", "pnpm", "bun", "deno"],
  },
  {
    id: "python",
    label: "Python",
    icon: Terminal,
    ids: ["pip", "pip_alt", "uv", "poetry", "pipenv_venv", "conda"],
  },
  { id: "jvm", label: "JVM", icon: Layers, ids: ["gradle", "gradle_wrap", "maven", "sbt", "ivy"] },
  { id: "swift", label: "iOS / Swift", icon: Shield, ids: ["cocoapods", "carthage", "swiftpm"] },
  { id: "ruby", label: "Ruby", icon: Gem, ids: ["rubygems", "bundler", "rbenv", "rvm"] },
  {
    id: "testing",
    label: "Tests",
    icon: FlaskConical,
    ids: ["playwright", "cypress", "puppeteer", "prisma"],
  },
  {
    id: "gamedev",
    label: "Game Engines",
    icon: Gamepad2,
    ids: ["unity_cache", "unity_hub", "godot"],
  },
  { id: "go", label: "Go", icon: Code2, ids: ["go_build", "go_mod"] },
  {
    id: "cloud",
    label: "Cloud & DevOps",
    icon: Cloud,
    ids: ["aws_cli", "docker_buildx", "terraform", "bazel"],
  },
  {
    id: "misc",
    label: "Autres",
    icon: Package,
    ids: ["cargo_reg", "cargo_git", "composer", "pub", "brew_cache", "android_avd"],
  },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} Go`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} Mo`;
  if (b >= 1e3) return `${Math.round(b / 1e3)} Ko`;
  return `${b} o`;
}

function fmtMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} Go`;
  if (mb > 0) return `${mb.toFixed(0)} Mo`;
  return "—";
}

function shortPath(full: string): string {
  const home = full.match(/^\/Users\/[^/]+/)?.[0] ?? "";
  return home ? full.replace(home, "~") : full;
}

const IMAGE_EXTS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "heic",
  "heif",
  "webp",
  "bmp",
  "tiff",
  "tif",
  "avif",
  "cr2",
  "nef",
  "arw",
]);
function isImage(path: string): boolean {
  return IMAGE_EXTS.has(path.split(".").pop()?.toLowerCase() ?? "");
}

function ImageThumb({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null | false>(null); // null=loading, false=failed
  useEffect(() => {
    let cancelled = false;
    invoke<string | null>("read_image_preview", { path })
      .then((v) => {
        if (!cancelled) setSrc(v ?? false);
      })
      .catch(() => {
        if (!cancelled) setSrc(false);
      });
    return () => {
      cancelled = true;
    };
  }, [path]);
  if (!src) return null;
  return (
    <img
      src={src}
      alt=""
      onError={() => setSrc(false)}
      className="rounded object-cover shrink-0"
      style={{ width: 40, height: 40, border: "1px solid var(--border)" }}
    />
  );
}

// ── Shared components ─────────────────────────────────────────────────────────

function SelectRow({
  checked,
  onToggle,
  leftIcon,
  label,
  sub,
  rightBadge,
}: {
  checked: boolean;
  onToggle: () => void;
  leftIcon: React.ReactNode;
  label: string;
  sub?: React.ReactNode;
  rightBadge: string;
}) {
  return (
    <button
      onClick={onToggle}
      className="w-full flex items-center gap-3 px-4 py-3 text-left transition-colors"
      style={{ borderLeft: checked ? "2px solid var(--accent)" : "2px solid transparent" }}
    >
      {checked ? (
        <Trash2 size={16} style={{ color: "var(--accent)", flexShrink: 0 }} />
      ) : (
        <Circle size={16} style={{ color: "var(--text-3)", flexShrink: 0 }} />
      )}
      {leftIcon}
      <span className="text-sm font-medium shrink-0" style={{ color: "var(--text-1)" }}>
        {label}
      </span>
      {sub && (
        <span className="text-[11px] truncate min-w-0" style={{ color: "var(--text-3)" }}>
          {sub}
        </span>
      )}
      <span
        className="text-[11px] font-mono shrink-0 ml-auto px-2 py-0.5 rounded"
        style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
      >
        {rightBadge}
      </span>
    </button>
  );
}

function TabHeader({
  summary,
  onSelectAll,
  allLabel,
}: {
  summary: string;
  onSelectAll: () => void;
  allLabel: string;
}) {
  return (
    <div className="flex items-center justify-between px-1">
      <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
        {summary}
      </span>
      <button
        onClick={onSelectAll}
        className="text-[11px] font-medium"
        style={{ color: "var(--accent)" }}
      >
        {allLabel}
      </button>
    </div>
  );
}

function ActionBtn({
  count,
  bytes,
  running,
  label,
  onRun,
}: {
  count: number;
  bytes: number;
  running: boolean;
  label?: string;
  onRun: () => void;
}) {
  return (
    <button
      onClick={onRun}
      disabled={running || count === 0}
      className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
    >
      {running ? (
        <>
          <Loader2 size={14} className="animate-spin" /> {label ?? "Suppression…"}
        </>
      ) : (
        <>
          <Trash2 size={14} /> {label ?? "Déplacer dans la corbeille"} ({count} · {fmtBytes(bytes)})
        </>
      )}
    </button>
  );
}

function GroupRow({
  label,
  icon: Icon,
  count,
  bytes,
  maxRisk,
  expanded,
  onToggle,
}: {
  label: string;
  icon: React.ElementType;
  count: number;
  bytes: number;
  maxRisk: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const color = (RISK_COLOR as readonly string[])[maxRisk] ?? "var(--success)";
  return (
    <button
      onClick={onToggle}
      className="w-full flex items-center gap-3 px-4 py-3 text-left transition-opacity hover:opacity-80"
    >
      {expanded ? (
        <ChevronDown size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
      ) : (
        <ChevronRight size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
      )}
      <Icon size={15} style={{ color, flexShrink: 0 }} />
      <span className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
        {label}
      </span>
      <span
        className="text-[10px] px-1.5 py-0.5 rounded-full font-medium ml-0.5"
        style={{
          background: "var(--bg)",
          color: "var(--text-3)",
          border: "1px solid var(--border)",
        }}
      >
        {count}
      </span>
      <span
        className="ml-auto text-[11px] font-mono shrink-0 px-2 py-0.5 rounded"
        style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
      >
        {fmtBytes(bytes)}
      </span>
    </button>
  );
}

// ── Tab 0: Aperçu ─────────────────────────────────────────────────────────────

function ApercuTab({
  caches,
  cachesLoading,
  onCleanSafe,
  onOpenTab,
}: {
  caches: DevCache[] | null;
  cachesLoading: boolean;
  onCleanSafe: () => void;
  onOpenTab: (tab: number) => void;
}) {
  const [metrics, setMetrics] = useState<DiskMetrics | null>(null);
  const [forecast, setForecast] = useState<number | null | undefined>(undefined);

  useEffect(() => {
    invoke<DiskMetrics>("get_quick_metrics")
      .then(setMetrics)
      .catch((e) => console.error("get_quick_metrics:", e));
    invoke<number | null>("get_disk_forecast")
      .then(setForecast)
      .catch(() => setForecast(null));
  }, []);

  const usedPct = metrics ? metrics.disk_used_percent : 0;
  const barColor = usedPct > 85 ? "var(--danger)" : usedPct > 70 ? "var(--warning)" : "var(--info)";

  const safeBytes = caches?.filter((c) => c.risk === 0).reduce((s, c) => s + c.size_bytes, 0) ?? 0;
  const cautionBytes =
    caches?.filter((c) => c.risk === 1).reduce((s, c) => s + c.size_bytes, 0) ?? 0;
  const riskyBytes = caches?.filter((c) => c.risk === 2).reduce((s, c) => s + c.size_bytes, 0) ?? 0;
  const totalReclaimable = safeBytes + cautionBytes + riskyBytes;
  const largeFilesBytes =
    _cache.largeFiles?.reduce((sum, file) => sum + file.size_bytes, 0) ?? null;
  const binaryBytes =
    _cache.binaries
      ?.filter((entry) => !entry.thinning_unsafe)
      .reduce((sum, entry) => sum + entry.reclaimable_bytes, 0) ?? null;

  const priorities = [
    {
      id: "safe",
      title: "Nettoyage recommandé",
      description:
        safeBytes > 0
          ? `${caches?.filter((cache) => cache.risk === 0).length ?? 0} éléments sans risque identifiés`
          : "Aucun cache sûr à nettoyer pour le moment",
      value: safeBytes > 0 ? fmtBytes(safeBytes) : "À jour",
      color: "var(--success)",
      icon: Shield,
      tab: 2,
    },
    {
      id: "review",
      title: "Éléments à vérifier",
      description:
        cautionBytes + riskyBytes > 0
          ? "Vérifiez leur contenu avant de les placer dans la Corbeille"
          : "Aucun élément sensible en attente",
      value:
        cautionBytes + riskyBytes > 0 ? fmtBytes(cautionBytes + riskyBytes) : "Rien à signaler",
      color: cautionBytes + riskyBytes > 0 ? "var(--warning)" : "var(--text-3)",
      icon: AlertTriangle,
      tab: 2,
    },
    {
      id: "large-files",
      title: "Fichiers volumineux",
      description:
        largeFilesBytes === null
          ? "Repérez les fichiers qui occupent réellement votre stockage"
          : `${_cache.largeFiles?.length ?? 0} fichiers de plus de 100 Mo analysés`,
      value: largeFilesBytes === null ? "Analyser" : fmtBytes(largeFilesBytes),
      color: "var(--info)",
      icon: FileText,
      tab: 3,
    },
    {
      id: "duplicates",
      title: "Doublons",
      description: "Comparez les fichiers identiques avant toute suppression",
      value: "Rechercher",
      color: "var(--violet)",
      icon: Copy,
      tab: 4,
    },
    {
      id: "binaries",
      title: "Binaires universels",
      description:
        binaryBytes === null
          ? "Vérifiez les applications compatibles avec Apple Silicon"
          : `${_cache.binaries?.filter((entry) => !entry.thinning_unsafe).length ?? 0} applications compatibles détectées`,
      value: binaryBytes === null ? "Vérifier" : `~${fmtBytes(binaryBytes)}`,
      color: "var(--accent)",
      icon: Cpu,
      tab: 6,
    },
  ];

  function fmtForecast(days: number): string {
    if (days <= 7) return `dans ~${days} jour${days > 1 ? "s" : ""}`;
    if (days <= 21)
      return `dans ~${Math.round(days / 7)} semaine${Math.round(days / 7) > 1 ? "s" : ""}`;
    if (days <= 90) return `dans ~${Math.round(days / 7)} semaines`;
    if (days <= 365) return `dans ~${Math.round(days / 30)} mois`;
    return `dans ~${Math.round(days / 365)} an${Math.round(days / 365) > 1 ? "s" : ""}`;
  }

  return (
    <div className="flex flex-col gap-2">
      {/* ── Hero + Disque côte à côte ── */}
      <div className="flex gap-2">
        {/* Espace récupérable */}
        <div className="card flex-1 px-3 py-2.5">
          {cachesLoading ? (
            <div className="flex items-center gap-1.5">
              <Loader2 size={12} className="animate-spin" style={{ color: "var(--accent)" }} />
              <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
                Calcul…
              </span>
            </div>
          ) : (
            <>
              <div className="text-[11px] mb-0.5" style={{ color: "var(--text-3)" }}>
                Récupérables
              </div>
              <div className="text-xl font-bold leading-none" style={{ color: "var(--text-1)" }}>
                {fmtBytes(totalReclaimable)}
              </div>
              {totalReclaimable > 0 && (
                <>
                  <div
                    className="h-1.5 rounded-full overflow-hidden flex mt-2"
                    style={{ background: "var(--bar-track)" }}
                  >
                    {safeBytes > 0 && (
                      <motion.div
                        initial={{ width: 0 }}
                        animate={{ width: `${(safeBytes / totalReclaimable) * 100}%` }}
                        transition={{ duration: 0.6 }}
                        className="h-full"
                        style={{ background: "var(--success)" }}
                      />
                    )}
                    {cautionBytes > 0 && (
                      <motion.div
                        initial={{ width: 0 }}
                        animate={{ width: `${(cautionBytes / totalReclaimable) * 100}%` }}
                        transition={{ duration: 0.6, delay: 0.05 }}
                        className="h-full"
                        style={{ background: "var(--warning)" }}
                      />
                    )}
                    {riskyBytes > 0 && (
                      <motion.div
                        initial={{ width: 0 }}
                        animate={{ width: `${(riskyBytes / totalReclaimable) * 100}%` }}
                        transition={{ duration: 0.6, delay: 0.1 }}
                        className="h-full"
                        style={{ background: "var(--danger)" }}
                      />
                    )}
                  </div>
                  <div className="flex items-center gap-2 mt-1 text-[9px]">
                    {safeBytes > 0 && (
                      <span style={{ color: "var(--success)" }}>{fmtBytes(safeBytes)} sûr</span>
                    )}
                    {cautionBytes > 0 && (
                      <span style={{ color: "var(--warning)" }}>{fmtBytes(cautionBytes)} ⚠</span>
                    )}
                    {riskyBytes > 0 && (
                      <span style={{ color: "var(--danger)" }}>{fmtBytes(riskyBytes)} !</span>
                    )}
                  </div>
                  <button
                    onClick={onCleanSafe}
                    className="btn-primary w-full flex items-center justify-center gap-1 mt-2 py-1 text-[11px]"
                  >
                    <Trash2 size={11} /> Nettoyer les sûrs
                  </button>
                </>
              )}
            </>
          )}
        </div>

        {/* Disque */}
        <div className="card flex-1 px-3 py-2.5">
          <span className="text-[11px] font-semibold" style={{ color: "var(--text-1)" }}>
            Macintosh HD
          </span>
          <div className="text-[11px] mt-1 mb-1" style={{ color: "var(--text-3)" }}>
            {metrics ? `${fmtBytes(metrics.disk_used)} / ${fmtBytes(metrics.disk_total)}` : "…"}
          </div>
          <div
            className="h-1.5 rounded-full overflow-hidden"
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
          <div className="flex justify-between mt-1 text-[9px]" style={{ color: "var(--text-3)" }}>
            <span>
              {metrics ? fmtBytes(metrics.disk_total - metrics.disk_used) + " libre" : ""}
            </span>
            <span>{metrics ? Math.round(usedPct) + "% utilisé" : ""}</span>
          </div>
          {forecast != null && forecast !== undefined && (
            <div
              className="mt-1 text-[9px] flex items-center gap-1"
              style={{
                color:
                  forecast <= 30
                    ? "var(--danger)"
                    : forecast <= 90
                      ? "var(--warning)"
                      : "var(--text-3)",
              }}
            >
              <AlertTriangle size={9} />
              Plein {fmtForecast(forecast)}
            </div>
          )}
        </div>
      </div>

      {/* ── Centre de priorités ── */}
      <div className="flex items-end justify-between px-1 mt-1">
        <div>
          <span
            className="text-[10px] uppercase tracking-widest font-semibold block"
            style={{ color: "var(--text-3)" }}
          >
            Centre de priorités
          </span>
          <span className="text-[10px]" style={{ color: "var(--text-3)" }}>
            Les actions utiles, sans analyse récursive automatique
          </span>
        </div>
      </div>
      <div className="grid grid-cols-2 gap-2">
        {priorities.map((priority, index) => {
          const PriorityIcon = priority.icon;
          return (
            <motion.button
              key={priority.id}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: index * 0.035 }}
              onClick={() => onOpenTab(priority.tab)}
              className="card px-3 py-2.5 flex items-center gap-3 text-left transition-opacity hover:opacity-80"
            >
              <div
                className="w-8 h-8 rounded-xl flex items-center justify-center shrink-0"
                style={{
                  color: priority.color,
                  background: `color-mix(in srgb, ${priority.color} 11%, transparent)`,
                }}
              >
                <PriorityIcon size={14} />
              </div>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span
                    className="text-[11px] font-semibold truncate"
                    style={{ color: "var(--text-1)" }}
                  >
                    {priority.title}
                  </span>
                  <span
                    className="text-[10px] font-mono ml-auto shrink-0"
                    style={{ color: priority.color }}
                  >
                    {priority.value}
                  </span>
                </div>
                <span
                  className="text-[9px] block truncate mt-0.5"
                  style={{ color: "var(--text-3)" }}
                >
                  {priority.description}
                </span>
              </div>
              <ChevronRight size={11} style={{ color: "var(--text-3)", flexShrink: 0 }} />
            </motion.button>
          );
        })}
      </div>
    </div>
  );
}

// ── Tab 1: Nettoyer ───────────────────────────────────────────────────────────

function NettoyerTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const { t } = useT();
  const [sizes, setSizes] = useState<Record<string, number>>(_cache.cleanSizes ?? {});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [installers, setInstallers] = useState<InstallerFile[]>(_cache.cleanInstallers ?? []);
  const [selectedInst, setSelectedInst] = useState<Set<string>>(new Set());
  const [showInst, setShowInst] = useState(false);
  const [scanning, setScanning] = useState(!_cache.cleanSizesLoaded);
  const mo = useMo();

  const loadCleanSizes = useCallback(async () => {
    setScanning(true);
    const [cats, insts] = await Promise.all([
      invoke<CleanCategorySize[]>("get_clean_sizes").catch(() => [] as CleanCategorySize[]),
      invoke<InstallerFile[]>("list_installer_files").catch(() => [] as InstallerFile[]),
    ]);
    const map: Record<string, number> = {};
    for (const c of cats) map[c.id] = c.size_mb;
    _cache.cleanSizes = map;
    _cache.cleanInstallers = insts;
    _cache.cleanSizesLoaded = true;
    setSizes(map);
    setInstallers(insts);
    setScanning(false);
  }, []);

  useEffect(() => {
    if (!_cache.cleanSizesLoaded) loadCleanSizes();
  }, [loadCleanSizes]);

  const isRunning = mo.status === "running";
  const installersTotal = installers.reduce((s, i) => s + i.size_bytes, 0) / 1_048_576;
  const totalMb =
    CLEAN_CATS.filter((c) => selected.has(c.id)).reduce((s, c) => s + (sizes[c.id] || 0), 0) +
    Array.from(selectedInst).reduce(
      (s, p) => s + (installers.find((i) => i.path === p)?.size_bytes || 0) / 1_048_576,
      0
    );
  const totalCount = selected.size + selectedInst.size;
  const allSelected =
    selected.size === CLEAN_CATS.length && selectedInst.size === installers.length;

  const toggle = (id: string) =>
    setSelected((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const toggleInst = (path: string) =>
    setSelectedInst((p) => {
      const n = new Set(p);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });

  const handleRun = async () => {
    mo.reset();
    await mo.runCmd("run_clean_selection", {
      categories: Array.from(selected),
      installerPaths: Array.from(selectedInst),
    });
    const ok = mo.status !== "error";
    onToast(ok, ok ? "Nettoyage terminé avec succès" : (mo.error ?? "Erreur lors du nettoyage"));
    setSelected(new Set());
    setSelectedInst(new Set());
    loadCleanSizes();
  };

  return (
    <div className="flex flex-col gap-2 relative">
      {isRunning && (
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
            Nettoyage en cours…
          </span>
        </div>
      )}
      {scanning ? (
        <div className="flex flex-col items-center justify-center h-32 gap-3">
          <Loader2 size={20} className="animate-spin" style={{ color: "var(--accent)" }} />
          <span className="text-xs" style={{ color: "var(--text-3)" }}>
            Analyse des tailles…
          </span>
        </div>
      ) : (
        <>
          <TabHeader
            summary={
              totalCount > 0 ? `${totalCount} · ${totalMb.toFixed(0)} Mo` : t.common_none_selected
            }
            onSelectAll={() => {
              if (allSelected) {
                setSelected(new Set());
                setSelectedInst(new Set());
              } else {
                setSelected(new Set(CLEAN_CATS.map((c) => c.id)));
                setSelectedInst(new Set(installers.map((i) => i.path)));
              }
            }}
            allLabel={allSelected ? t.common_deselect_all : t.common_select_all}
          />
          <div className="card overflow-hidden">
            {CLEAN_CATS.map((cat, i) => (
              <div key={cat.id} style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}>
                <SelectRow
                  checked={selected.has(cat.id)}
                  onToggle={() => toggle(cat.id)}
                  leftIcon={
                    <cat.icon size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                  }
                  label={cat.label}
                  sub={cat.desc}
                  rightBadge={fmtMb(sizes[cat.id] || 0)}
                />
              </div>
            ))}
          </div>

          <div className="card overflow-hidden">
            <button
              onClick={() => setShowInst((v) => !v)}
              className="w-full flex items-center gap-3 px-4 py-3 text-left"
              style={{ borderLeft: "2px solid transparent" }}
            >
              {showInst ? (
                <ChevronDown size={14} style={{ color: "var(--text-3)" }} />
              ) : (
                <ChevronRight size={14} style={{ color: "var(--text-3)" }} />
              )}
              <span className="text-sm font-medium shrink-0" style={{ color: "var(--text-1)" }}>
                {t.clean_installers_section} ({installers.length})
              </span>
              <span className="text-[11px] truncate min-w-0" style={{ color: "var(--text-3)" }}>
                {installers.length > 0
                  ? `${fmtMb(installersTotal)} ${t.clean_installers_total}`
                  : t.clean_installers_none}
              </span>
              {installers.length > 0 && (
                <span
                  className="text-[11px] font-mono shrink-0 px-2 py-0.5 rounded"
                  style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
                >
                  {fmtMb(installersTotal)}
                </span>
              )}
            </button>
            <AnimatePresence>
              {showInst && (
                <motion.div
                  initial={{ height: 0, opacity: 0 }}
                  animate={{ height: "auto", opacity: 1 }}
                  exit={{ height: 0, opacity: 0 }}
                  className="overflow-hidden"
                >
                  {installers.map((inst) => (
                    <div key={inst.path} style={{ borderTop: "1px solid var(--border)" }}>
                      <SelectRow
                        checked={selectedInst.has(inst.path)}
                        onToggle={() => toggleInst(inst.path)}
                        leftIcon={
                          <FileText size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                        }
                        label={inst.name}
                        sub={inst.source}
                        rightBadge={fmtMb(inst.size_bytes / 1_048_576)}
                      />
                    </div>
                  ))}
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </>
      )}

      <button
        onClick={handleRun}
        disabled={isRunning || totalCount === 0}
        className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
      >
        {isRunning ? (
          <>
            <Loader2 size={14} className="animate-spin" /> {t.clean_running}
          </>
        ) : (
          <>
            <Trash2 size={14} /> {t.clean_run} ({totalCount} · {totalMb.toFixed(0)} Mo)
          </>
        )}
      </button>
    </div>
  );
}

// ── Tab 2: Développeur ────────────────────────────────────────────────────────

function DeveloppeurTab({
  caches,
  cachesLoading,
  onReload,
  selectSafeTrigger,
  onToast,
}: {
  caches: DevCache[] | null;
  cachesLoading: boolean;
  onReload: () => Promise<void>;
  selectSafeTrigger: number;
  onToast: (ok: boolean, msg: string) => void;
}) {
  const [artifacts, setArtifacts] = useState<ProjectArtifact[] | null>(_cache.artifacts ?? null);
  const [artLoad, setArtLoad] = useState(!_cache.artifacts);
  const [derivedProjects, setDerivedProjects] = useState<DerivedDataProject[] | null>(
    _cache.derivedProjects ?? null
  );
  const [showDerived, setShowDerived] = useState(false);
  const [xcodeRunning, setXcodeRunning] = useState(false);
  const [selectedC, setSelectedC] = useState<Set<string>>(new Set());
  const [selectedA, setSelectedA] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (selectSafeTrigger > 0 && caches) {
      setSelectedC(new Set(caches.filter((c) => c.risk === 0).map((c) => c.id)));
    }
  }, [selectSafeTrigger, caches]);

  const loadArtifacts = useCallback(async () => {
    setArtLoad(true);
    try {
      const data = await invoke<ProjectArtifact[]>("get_project_artifacts");
      _cache.artifacts = data;
      setArtifacts(data);
    } catch (e) {
      console.error("get_project_artifacts:", e);
    }
    setArtLoad(false);
  }, []);

  useEffect(() => {
    if (!_cache.artifacts) loadArtifacts();
    if (!_cache.derivedProjects) {
      invoke<DerivedDataProject[]>("get_derived_data_projects")
        .then((data) => {
          _cache.derivedProjects = data;
          setDerivedProjects(data);
        })
        .catch((e) => console.error("get_derived_data_projects:", e));
    }
  }, [loadArtifacts]);

  useEffect(() => {
    const xcodeIds = [
      "xcode_dd",
      "xcode_devsup",
      "xcode_logs",
      "xcode_prev",
      "xcode_arch",
      "ios_sim_cache",
      "ios_sim_dev",
    ];
    if (xcodeIds.some((id) => selectedC.has(id))) {
      invoke<boolean>("is_xcode_running")
        .then(setXcodeRunning)
        .catch((e) => console.error("is_xcode_running:", e));
    } else {
      setXcodeRunning(false);
    }
  }, [selectedC]);

  const cacheMap = useMemo(() => {
    const m = new Map<string, DevCache>();
    for (const c of caches ?? []) m.set(c.id, c);
    return m;
  }, [caches]);

  const resolvedGroups = useMemo(
    () =>
      CACHE_GROUPS.map((g) => ({
        ...g,
        present: g.ids.map((id) => cacheMap.get(id)).filter(Boolean) as DevCache[],
      })).filter((g) => g.present.length >= 2),
    [cacheMap]
  );

  const groupedIds = useMemo(
    () => new Set(resolvedGroups.flatMap((g) => g.present.map((c) => c.id))),
    [resolvedGroups]
  );
  const loneCaches = useMemo(
    () => (caches ?? []).filter((c) => !groupedIds.has(c.id)),
    [caches, groupedIds]
  );

  const selCaches = (caches ?? []).filter((c) => selectedC.has(c.id));
  const selArts = (artifacts ?? []).filter((a) => selectedA.has(a.artifact_path));
  const totalCount = selCaches.length + selArts.length;
  const totalBytes =
    selCaches.reduce((s, c) => s + c.size_bytes, 0) + selArts.reduce((s, a) => s + a.size_bytes, 0);
  const allCount = (caches?.length ?? 0) + (artifacts?.length ?? 0);
  const allSelected = totalCount === allCount && allCount > 0;

  const toggleC = (id: string) =>
    setSelectedC((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const toggleA = (path: string) =>
    setSelectedA((p) => {
      const n = new Set(p);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });
  const toggleGroup = (gid: string) =>
    setExpandedGroups((p) => {
      const n = new Set(p);
      if (n.has(gid)) n.delete(gid);
      else n.add(gid);
      return n;
    });

  const handleRun = async () => {
    setDeleting(true);
    const paths = [...selCaches.map((c) => c.path), ...selArts.map((a) => a.artifact_path)];
    setProgress({ done: 0, total: paths.length });
    for (let i = 0; i < paths.length; i++) {
      try {
        await invoke("move_to_trash", { path: paths[i] });
      } catch (e) {
        console.error("move_to_trash:", e);
      }
      setProgress({ done: i + 1, total: paths.length });
    }
    setProgress(null);
    setSelectedC(new Set());
    setSelectedA(new Set());
    await Promise.all([onReload(), loadArtifacts()]);
    invoke<DerivedDataProject[]>("get_derived_data_projects")
      .then((data) => {
        _cache.derivedProjects = data;
        setDerivedProjects(data);
      })
      .catch((e) => console.error("get_derived_data_projects:", e));
    setDeleting(false);
    onToast(
      true,
      `${paths.length} élément${paths.length > 1 ? "s" : ""} déplacé${paths.length > 1 ? "s" : ""} dans la corbeille`
    );
  };

  const hasDerivedData = (caches ?? []).some((c) => c.id === "xcode_dd");

  function CacheRow({ cache, border }: { cache: DevCache; border?: boolean }) {
    const RiskIcon = RISK_ICON[cache.risk];
    return (
      <div>
        <div style={border ? { borderTop: "1px solid var(--border)" } : {}}>
          <SelectRow
            checked={selectedC.has(cache.id)}
            onToggle={() => toggleC(cache.id)}
            leftIcon={
              <RiskIcon size={15} style={{ color: RISK_COLOR[cache.risk], flexShrink: 0 }} />
            }
            label={cache.name}
            sub={
              <>
                <span
                  className="inline-block px-1.5 py-0.5 rounded-full text-[10px] mr-1.5"
                  style={{
                    background: RISK_COLOR[cache.risk] + "18",
                    color: RISK_COLOR[cache.risk],
                  }}
                >
                  {RISK_LABEL[cache.risk]}
                </span>
                {cache.days_since_use >= 0 && `il y a ${cache.days_since_use}j`}
              </>
            }
            rightBadge={fmtBytes(cache.size_bytes)}
          />
          {cache.description && (
            <div
              className="px-4 pb-2 text-[10px] leading-relaxed"
              style={{ color: "var(--text-3)", marginTop: -4 }}
            >
              {cache.description}
            </div>
          )}
        </div>
        {cache.id === "xcode_dd" &&
          hasDerivedData &&
          derivedProjects &&
          derivedProjects.length > 0 && (
            <div style={{ borderTop: "1px solid var(--border)" }}>
              <button
                onClick={() => setShowDerived((v) => !v)}
                className="w-full flex items-center gap-2 px-4 py-2 text-left text-[11px]"
                style={{ color: "var(--text-3)", background: "var(--bg)" }}
              >
                {showDerived ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                DerivedData par projet ({derivedProjects.length})
              </button>
              <AnimatePresence>
                {showDerived && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: "auto", opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    className="overflow-hidden"
                  >
                    {derivedProjects.map((proj, pi) => (
                      <div
                        key={proj.path}
                        className="flex items-center gap-3 px-6 py-1.5 text-xs"
                        style={{
                          borderTop: pi > 0 ? "1px solid var(--border)" : undefined,
                          color: "var(--text-2)",
                        }}
                      >
                        <span className="flex-1 truncate font-medium">{proj.name}</span>
                        {proj.workspace_path && (
                          <span
                            className="truncate text-[10px] max-w-[140px]"
                            style={{ color: "var(--text-3)" }}
                          >
                            {proj.workspace_path.split("/").slice(-2).join("/")}
                          </span>
                        )}
                        <span
                          className="shrink-0 font-mono text-[11px]"
                          style={{ color: "var(--text-3)" }}
                        >
                          {fmtBytes(proj.size_bytes)}
                        </span>
                      </div>
                    ))}
                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <TabHeader
        summary={totalCount > 0 ? `${totalCount} · ${fmtBytes(totalBytes)}` : "Aucune sélection"}
        onSelectAll={() => {
          if (allSelected) {
            setSelectedC(new Set());
            setSelectedA(new Set());
          } else {
            setSelectedC(new Set(caches?.map((c) => c.id) ?? []));
            setSelectedA(new Set(artifacts?.map((a) => a.artifact_path) ?? []));
          }
        }}
        allLabel={allSelected ? "Tout désélectionner" : "Tout sélectionner"}
      />

      {xcodeRunning && (
        <div
          className="flex items-center gap-2 px-3 py-2 rounded-lg text-xs"
          style={{
            background: "var(--warning-dim)",
            color: "var(--warning)",
            border: "1px solid var(--warning-soft)",
          }}
        >
          <AlertTriangle size={13} />
          Xcode est ouvert — fermez-le avant de déplacer ses caches dans la Corbeille pour éviter
          des incohérences.
        </div>
      )}

      {cachesLoading ? (
        <div className="flex items-center gap-2 py-3 px-1" style={{ color: "var(--text-3)" }}>
          <Loader2 size={13} className="animate-spin" />
          <span className="text-xs">Analyse des caches…</span>
        </div>
      ) : (
        <>
          {resolvedGroups.map((group) => {
            const isExpanded = expandedGroups.has(group.id);
            const groupBytes = group.present.reduce((s, c) => s + c.size_bytes, 0);
            const maxRisk = Math.max(...group.present.map((c) => c.risk)) as 0 | 1 | 2;
            return (
              <div key={group.id} className="card overflow-hidden">
                <GroupRow
                  label={group.label}
                  icon={group.icon}
                  count={group.present.length}
                  bytes={groupBytes}
                  maxRisk={maxRisk}
                  expanded={isExpanded}
                  onToggle={() => toggleGroup(group.id)}
                />
                <AnimatePresence>
                  {isExpanded && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: "auto", opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      className="overflow-hidden"
                    >
                      {group.present.map((cache) => (
                        <div key={cache.id} style={{ borderTop: "1px solid var(--border)" }}>
                          <CacheRow cache={cache} />
                        </div>
                      ))}
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            );
          })}

          {loneCaches.length > 0 && (
            <div className="card overflow-hidden">
              {loneCaches.map((cache, i) => (
                <div key={cache.id} style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}>
                  <CacheRow cache={cache} />
                </div>
              ))}
            </div>
          )}
        </>
      )}

      {artLoad ? (
        <div className="flex items-center gap-2 py-3 px-1" style={{ color: "var(--text-3)" }}>
          <Loader2 size={13} className="animate-spin" />
          <span className="text-xs">Analyse des projets…</span>
        </div>
      ) : (
        (artifacts?.length ?? 0) > 0 && (
          <div className="card overflow-hidden">
            {artifacts!.map((a, i) => (
              <div
                key={a.artifact_path}
                style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}
              >
                <SelectRow
                  checked={selectedA.has(a.artifact_path)}
                  onToggle={() => toggleA(a.artifact_path)}
                  leftIcon={<Package size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />}
                  label={a.project_name}
                  sub={a.artifact_type}
                  rightBadge={fmtBytes(a.size_bytes)}
                />
              </div>
            ))}
          </div>
        )
      )}

      {!cachesLoading &&
        !artLoad &&
        (caches?.length ?? 0) === 0 &&
        (artifacts?.length ?? 0) === 0 && (
          <div className="text-center py-10 text-sm" style={{ color: "var(--text-3)" }}>
            Aucun élément trouvé
          </div>
        )}

      <ActionBtn
        count={totalCount}
        bytes={totalBytes}
        running={deleting}
        onRun={handleRun}
        label={progress ? `Suppression… (${progress.done}/${progress.total})` : undefined}
      />
    </div>
  );
}

// ── Tab 3: Fichiers lourds ────────────────────────────────────────────────────

function FichiersTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [files, setFiles] = useState<LargeFile[] | null>(_cache.largeFiles ?? null);
  const [loading, setLoading] = useState(!_cache.largeFiles);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [expandedFolders, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<LargeFile[]>("get_large_files");
      _cache.largeFiles = data;
      setFiles(data);
    } catch (e) {
      console.error("get_large_files:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    if (!_cache.largeFiles) load();
  }, [load]);

  const folderGroups = useMemo(() => {
    if (!files) return [];
    const groups = new Map<string, { label: string; files: LargeFile[] }>();
    for (const f of files) {
      const parts = f.path.replace(/^\/Users\/[^/]+/, "~").split("/");
      const key = parts.slice(0, -1).join("/") || "~";
      const label = parts[parts.length - 2] || "~";
      if (!groups.has(key)) groups.set(key, { label, files: [] });
      groups.get(key)!.files.push(f);
    }
    return Array.from(groups.entries())
      .map(([key, g]) => ({ key, ...g, totalBytes: g.files.reduce((s, f) => s + f.size_bytes, 0) }))
      .sort((a, b) => b.totalBytes - a.totalBytes);
  }, [files]);

  const toggle = (path: string) =>
    setSelected((p) => {
      const n = new Set(p);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });
  const toggleFolder = (key: string) =>
    setExpanded((p) => {
      const n = new Set(p);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const selFiles = (files ?? []).filter((f) => selected.has(f.path));
  const totalCount = selFiles.length;
  const totalBytes = selFiles.reduce((s, f) => s + f.size_bytes, 0);
  const allSelected = (files?.length ?? 0) > 0 && selected.size === (files?.length ?? 0);

  const handleRun = async () => {
    setDeleting(true);
    const paths = selFiles.map((f) => f.path);
    setProgress({ done: 0, total: paths.length });
    for (let i = 0; i < paths.length; i++) {
      try {
        await invoke("move_to_trash", { path: paths[i] });
      } catch (e) {
        console.error("move_to_trash:", e);
      }
      setProgress({ done: i + 1, total: paths.length });
    }
    setProgress(null);
    setSelected(new Set());
    await load();
    setDeleting(false);
    onToast(
      true,
      `${paths.length} fichier${paths.length > 1 ? "s" : ""} déplacé${paths.length > 1 ? "s" : ""} dans la corbeille`
    );
  };

  return (
    <div className="flex flex-col gap-2">
      <TabHeader
        summary={
          loading
            ? ""
            : (files?.length ?? 0) === 0
              ? "Aucun fichier >100 Mo"
              : totalCount > 0
                ? `${files!.length} fichier${files!.length > 1 ? "s" : ""} · ${totalCount} sélectionné · ${fmtBytes(totalBytes)}`
                : `${files!.length} fichier${files!.length > 1 ? "s" : ""} >100 Mo`
        }
        onSelectAll={() => {
          if (allSelected) setSelected(new Set());
          else setSelected(new Set(files?.map((f) => f.path) ?? []));
        }}
        allLabel={allSelected ? "Tout désélectionner" : "Tout sélectionner"}
      />

      {loading ? (
        <div
          className="flex items-center justify-center py-10 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Recherche des grands fichiers…</span>
        </div>
      ) : (files?.length ?? 0) === 0 ? (
        <div className="text-center py-10 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun fichier &gt;100 Mo trouvé
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {folderGroups.map((group) => {
            const isExpanded = expandedFolders.has(group.key);
            return (
              <div key={group.key} className="card overflow-hidden">
                <button
                  onClick={() => toggleFolder(group.key)}
                  className="w-full flex items-center gap-3 px-4 py-3 text-left"
                >
                  {isExpanded ? (
                    <ChevronDown size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                  ) : (
                    <ChevronRight size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                  )}
                  <div
                    className="w-8 h-8 rounded-lg flex items-center justify-center shrink-0"
                    style={{ background: "var(--info-dim)" }}
                  >
                    <FolderOpen size={15} style={{ color: "var(--info)" }} />
                  </div>
                  <div className="flex flex-col flex-1 text-left">
                    <span className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
                      {group.label}
                    </span>
                    <span className="text-[10px]" style={{ color: "var(--text-3)" }}>
                      {group.files.length} fichier{group.files.length > 1 ? "s" : ""}
                    </span>
                  </div>
                  <span
                    className="text-[11px] font-mono shrink-0 px-2 py-0.5 rounded"
                    style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
                  >
                    {fmtBytes(group.totalBytes)}
                  </span>
                </button>
                <AnimatePresence>
                  {isExpanded && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: "auto", opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      className="overflow-hidden"
                    >
                      {group.files.map((f) => (
                        <div key={f.path} style={{ borderTop: "1px solid var(--border)" }}>
                          <SelectRow
                            checked={selected.has(f.path)}
                            onToggle={() => toggle(f.path)}
                            leftIcon={
                              <FileText
                                size={15}
                                style={{ color: "var(--text-3)", flexShrink: 0 }}
                              />
                            }
                            label={f.name}
                            sub={`il y a ${f.days_old}j`}
                            rightBadge={fmtBytes(f.size_bytes)}
                          />
                        </div>
                      ))}
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            );
          })}
        </div>
      )}

      <ActionBtn
        count={totalCount}
        bytes={totalBytes}
        running={deleting}
        onRun={handleRun}
        label={progress ? `Suppression… (${progress.done}/${progress.total})` : undefined}
      />
    </div>
  );
}

// ── Tab 4: Doublons ───────────────────────────────────────────────────────────

function DoublonsTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [groups, setGroups] = useState<DuplicateGroup[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);

  const totalWasted = groups?.reduce((s, g) => s + g.wasted_bytes, 0) ?? 0;
  const totalGroups = groups?.length ?? 0;
  const selPaths = Array.from(selected);
  const selBytes = selPaths.reduce((s, path) => {
    const g = groups?.find((g) => g.paths.includes(path));
    return s + (g?.size_bytes ?? 0);
  }, 0);

  const handleScan = async () => {
    setScanning(true);
    setGroups(null);
    setSelected(new Set());
    setExpanded(new Set());
    try {
      const result = await invoke<DuplicateGroup[]>("find_duplicates");
      setGroups(result);
      // Auto-select all copies except the shortest-path one per group
      const autoSelect = new Set<string>();
      for (const g of result) {
        const sorted = [...g.paths].sort((a, b) => a.length - b.length || a.localeCompare(b));
        for (const p of sorted.slice(1)) autoSelect.add(p);
      }
      setSelected(autoSelect);
      setExpanded(new Set(result.slice(0, 5).map((g) => g.hash)));
    } catch {
      setGroups([]);
    }
    setScanning(false);
  };

  const togglePath = (path: string) =>
    setSelected((p) => {
      const n = new Set(p);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });

  const toggleGroup = (hash: string) =>
    setExpanded((p) => {
      const n = new Set(p);
      if (n.has(hash)) n.delete(hash);
      else n.add(hash);
      return n;
    });

  const autoSelectGroup = (g: DuplicateGroup, e: React.MouseEvent) => {
    e.stopPropagation();
    const sorted = [...g.paths].sort((a, b) => a.length - b.length || a.localeCompare(b));
    setSelected((prev) => {
      const n = new Set(prev);
      for (const p of sorted.slice(1)) n.add(p);
      return n;
    });
  };

  const handleDelete = async () => {
    setDeleting(true);
    const count = selPaths.length;
    setProgress({ done: 0, total: count });
    for (let i = 0; i < selPaths.length; i++) {
      try {
        await invoke("move_to_trash", { path: selPaths[i] });
      } catch (e) {
        console.error("move_to_trash:", e);
      }
      setProgress({ done: i + 1, total: count });
    }
    setProgress(null);
    const deleted = new Set(selPaths);
    setGroups((prev) =>
      prev
        ? prev
            .map((g) => ({ ...g, paths: g.paths.filter((p) => !deleted.has(p)), wasted_bytes: 0 }))
            .map((g) => ({ ...g, wasted_bytes: g.size_bytes * Math.max(0, g.paths.length - 1) }))
            .filter((g) => g.paths.length > 1)
        : null
    );
    setSelected(new Set());
    setDeleting(false);
    onToast(
      true,
      `${count} doublon${count > 1 ? "s" : ""} déplacé${count > 1 ? "s" : ""} dans la Corbeille`
    );
  };

  // Idle state
  if (!scanning && groups === null) {
    return (
      <div className="flex flex-col items-center justify-center gap-5 py-12">
        <div
          className="w-16 h-16 rounded-2xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <ScanSearch size={28} style={{ color: "var(--text-3)" }} />
        </div>
        <div className="text-center">
          <p className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
            Trouver les doublons
          </p>
          <p
            className="text-[11px] mt-1.5 leading-relaxed max-w-[230px]"
            style={{ color: "var(--text-3)" }}
          >
            Identifie les fichiers identiques dans Documents, Téléchargements, Bureau, Photos,
            Vidéos et Musique
          </p>
        </div>
        <button onClick={handleScan} className="btn-primary flex items-center gap-2 px-5 py-2.5">
          <ScanSearch size={14} /> Lancer l'analyse
        </button>
        <p className="text-[10px]" style={{ color: "var(--text-3)" }}>
          Peut prendre quelques minutes selon la taille des dossiers
        </p>
      </div>
    );
  }

  // Scanning state
  if (scanning) {
    return (
      <div className="flex flex-col items-center justify-center gap-5 py-12">
        <div className="relative w-16 h-16">
          <div
            className="w-16 h-16 rounded-2xl flex items-center justify-center"
            style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
          >
            <ScanSearch size={26} style={{ color: "var(--accent)" }} />
          </div>
          <Loader2
            size={18}
            className="animate-spin absolute -top-1 -right-1"
            style={{ color: "var(--accent)" }}
          />
        </div>
        <div className="text-center">
          <p className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
            Analyse en cours…
          </p>
          <p className="text-[11px] mt-1" style={{ color: "var(--text-3)" }}>
            Calcul des empreintes de fichiers
          </p>
        </div>
        <div className="flex gap-1.5">
          {[0, 1, 2].map((i) => (
            <motion.div
              key={i}
              className="w-1.5 h-1.5 rounded-full"
              style={{ background: "var(--accent)" }}
              animate={{ opacity: [0.3, 1, 0.3] }}
              transition={{ duration: 1.2, repeat: Infinity, delay: i * 0.2 }}
            />
          ))}
        </div>
      </div>
    );
  }

  // No duplicates
  if (groups !== null && groups.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-12">
        <div
          className="w-14 h-14 rounded-2xl flex items-center justify-center"
          style={{ background: "var(--success-dim)", border: "1px solid var(--success-soft)" }}
        >
          <Copy size={22} style={{ color: "var(--success)" }} />
        </div>
        <div className="text-center">
          <p className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
            Aucun doublon trouvé
          </p>
          <p className="text-[11px] mt-1" style={{ color: "var(--text-3)" }}>
            Vos fichiers sont tous uniques
          </p>
        </div>
        <button onClick={handleScan} className="btn-primary flex items-center gap-2 px-4 py-2">
          <RefreshCw size={13} /> Relancer
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {/* Summary */}
      <div className="flex items-center justify-between px-1">
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {totalGroups} groupe{totalGroups !== 1 ? "s" : ""} · {fmtBytes(totalWasted)} récupérables
          {selPaths.length > 0 && (
            <span style={{ color: "var(--accent)" }}> · {selPaths.length} sélectionné</span>
          )}
        </span>
        <button
          onClick={handleScan}
          className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={10} /> Relancer
        </button>
      </div>

      {/* Duplicate groups */}
      <div className="flex flex-col gap-1.5">
        {groups!.map((g) => {
          const isExpanded = expanded.has(g.hash);
          const sorted = [...g.paths].sort((a, b) => a.length - b.length || a.localeCompare(b));
          const selInGroup = sorted.filter((p) => selected.has(p)).length;
          const allImages = sorted.every(isImage);

          return (
            <div key={g.hash} className="card overflow-hidden">
              {/* Group header */}
              <button
                onClick={() => toggleGroup(g.hash)}
                className="w-full flex items-center gap-3 px-4 py-3 text-left transition-opacity hover:opacity-80"
              >
                {isExpanded ? (
                  <ChevronDown size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                ) : (
                  <ChevronRight size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                )}
                {/* Icon or thumbnail */}
                {allImages ? (
                  <div className="flex gap-1 shrink-0">
                    {sorted.slice(0, 3).map((p, i) => (
                      <div
                        key={i}
                        className="rounded overflow-hidden"
                        style={{
                          width: 32,
                          height: 32,
                          background: "var(--bar-track)",
                          border: "1px solid var(--border)",
                          flexShrink: 0,
                        }}
                      >
                        <ImageThumb path={p} />
                      </div>
                    ))}
                  </div>
                ) : (
                  <div
                    className="w-8 h-8 rounded-lg flex items-center justify-center shrink-0"
                    style={{ background: "var(--warning-dim)" }}
                  >
                    <Copy size={14} style={{ color: "var(--warning)" }} />
                  </div>
                )}
                <div className="flex flex-col flex-1 min-w-0">
                  <span className="text-[11px] font-semibold" style={{ color: "var(--text-1)" }}>
                    {g.paths.length} copies identiques
                  </span>
                  <span className="text-[10px]" style={{ color: "var(--text-3)" }}>
                    {fmtBytes(g.size_bytes)} / copie · {fmtBytes(g.wasted_bytes)} récupérables
                  </span>
                </div>
                {selInGroup > 0 && (
                  <span
                    className="text-[10px] px-1.5 py-0.5 rounded-full font-medium shrink-0"
                    style={{ background: "var(--accent)", color: "#fff" }}
                  >
                    {selInGroup}
                  </span>
                )}
                <button
                  onClick={(e) => autoSelectGroup(g, e)}
                  className="text-[10px] px-2 py-1 rounded shrink-0 transition-opacity hover:opacity-70"
                  style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                >
                  Auto
                </button>
              </button>

              {/* Image strip when collapsed and it's images */}
              {!isExpanded && allImages && (
                <div
                  className="flex gap-1 px-4 pb-3 overflow-x-auto"
                  style={{ borderTop: "1px solid var(--border)" }}
                >
                  {sorted.map((p, i) => (
                    <div
                      key={i}
                      className="rounded overflow-hidden shrink-0"
                      style={{
                        width: 56,
                        height: 56,
                        background: "var(--bar-track)",
                        border: "1px solid var(--border)",
                      }}
                    >
                      <ImageThumb path={p} />
                    </div>
                  ))}
                </div>
              )}

              {/* File entries */}
              <AnimatePresence>
                {isExpanded && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: "auto", opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    className="overflow-hidden"
                  >
                    {sorted.map((path, pi) => {
                      const isFirst = pi === 0;
                      const isChecked = selected.has(path);
                      const fileName = path.split("/").pop() ?? path;
                      const dirPart = shortPath(path.substring(0, path.lastIndexOf("/")));
                      return (
                        <div key={path} style={{ borderTop: "1px solid var(--border)" }}>
                          <button
                            onClick={() => togglePath(path)}
                            className="w-full flex items-center gap-3 px-4 py-2.5 text-left"
                            style={{
                              borderLeft: isChecked
                                ? "2px solid var(--accent)"
                                : "2px solid transparent",
                            }}
                          >
                            {isChecked ? (
                              <Trash2 size={14} style={{ color: "var(--accent)", flexShrink: 0 }} />
                            ) : (
                              <Circle size={14} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                            )}
                            {/* Thumbnail in expanded view */}
                            {allImages && (
                              <div
                                className="rounded overflow-hidden shrink-0"
                                style={{
                                  width: 36,
                                  height: 36,
                                  background: "var(--bar-track)",
                                  border: "1px solid var(--border)",
                                }}
                              >
                                <ImageThumb path={path} />
                              </div>
                            )}
                            <div className="flex flex-col flex-1 min-w-0">
                              <div className="flex items-center gap-1.5">
                                <span
                                  className="text-[11px] font-medium truncate"
                                  style={{ color: "var(--text-1)" }}
                                >
                                  {fileName}
                                </span>
                                {isFirst && (
                                  <span
                                    className="text-[9px] px-1 py-0.5 rounded shrink-0"
                                    style={{
                                      background: "var(--success-dim)",
                                      color: "var(--success)",
                                    }}
                                  >
                                    original
                                  </span>
                                )}
                              </div>
                              <span
                                className="text-[10px] truncate"
                                style={{ color: "var(--text-3)" }}
                              >
                                {dirPart}
                              </span>
                            </div>
                          </button>
                        </div>
                      );
                    })}
                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          );
        })}
      </div>

      {/* Delete button */}
      <button
        onClick={handleDelete}
        disabled={deleting || selPaths.length === 0}
        className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
      >
        {deleting ? (
          <>
            <Loader2 size={14} className="animate-spin" />
            {progress ? `Suppression… (${progress.done}/${progress.total})` : "Suppression…"}
          </>
        ) : (
          <>
            <Trash2 size={14} /> Supprimer la sélection ({selPaths.length} · {fmtBytes(selBytes)})
          </>
        )}
      </button>
    </div>
  );
}

// ── Tab 5: Vie Privée ─────────────────────────────────────────────────────────

function PrivacyTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [items, setItems] = useState<PrivacyItem[] | null>(_cache.privacyItems ?? null);
  const [scanning, setScanning] = useState(!_cache.privacyItems);
  const [cleaning, setCleaning] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setScanning(true);
    try {
      const data = await invoke<PrivacyItem[]>("scan_privacy_items");
      _cache.privacyItems = data;
      setItems(data);
    } catch {
      setItems([]);
    }
    setScanning(false);
  }, []);

  useEffect(() => {
    if (!_cache.privacyItems) load();
  }, [load]);

  const toggle = (id: string) =>
    setSelected((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const selItems = (items ?? []).filter((i) => selected.has(i.id));
  const totalBytes = selItems.reduce((s, i) => s + i.size_bytes, 0);

  const handleClean = async () => {
    if (selItems.length === 0) return;
    setCleaning(true);
    try {
      const freed = await invoke<number>("clean_privacy_items", { ids: Array.from(selected) });
      setSelected(new Set());
      await load();
      onToast(true, `${fmtBytes(freed)} libérés`);
    } catch (e) {
      onToast(false, String(e));
    }
    setCleaning(false);
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between px-1">
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {scanning
            ? "Analyse…"
            : `${items?.length ?? 0} élément${(items?.length ?? 0) !== 1 ? "s" : ""}`}
          {totalBytes > 0 && (
            <span style={{ color: "var(--accent)" }}> · {fmtBytes(totalBytes)} sélectionné</span>
          )}
        </span>
        <button
          onClick={load}
          disabled={scanning}
          className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={10} /> Actualiser
        </button>
      </div>

      {scanning ? (
        <div
          className="flex items-center gap-2 py-6 justify-center"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={14} className="animate-spin" />
          <span className="text-xs">Analyse en cours…</span>
        </div>
      ) : (items?.length ?? 0) === 0 ? (
        <div className="text-center py-8 text-sm" style={{ color: "var(--text-3)" }}>
          Aucune donnée privée trouvée
        </div>
      ) : (
        <div className="card overflow-hidden">
          {items!.map((item, i) => (
            <div key={item.id} style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}>
              <SelectRow
                checked={selected.has(item.id)}
                onToggle={() => toggle(item.id)}
                leftIcon={<Eye size={15} style={{ color: "var(--info)", flexShrink: 0 }} />}
                label={item.label}
                sub={item.path.replace(/^\/Users\/[^/]+/, "~")}
                rightBadge={fmtBytes(item.size_bytes)}
              />
            </div>
          ))}
        </div>
      )}

      <button
        onClick={handleClean}
        disabled={cleaning || selItems.length === 0}
        className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
      >
        {cleaning ? (
          <>
            <Loader2 size={14} className="animate-spin" /> Nettoyage…
          </>
        ) : (
          <>
            <Trash2 size={14} /> Nettoyer la sélection ({selItems.length} · {fmtBytes(totalBytes)})
          </>
        )}
      </button>
    </div>
  );
}

// ── Tab 6: Binaires universels ────────────────────────────────────────────────

function BinairesTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [entries, setEntries] = useState<UniversalBinaryEntry[] | null>(_cache.binaries ?? null);
  const [scanning, setScanning] = useState(!_cache.binaries);
  const [confirmPath, setConfirmPath] = useState<string | null>(null);
  const [thinningPath, setThinningPath] = useState<string | null>(null);

  const load = useCallback(async () => {
    setScanning(true);
    try {
      const data = await invoke<UniversalBinaryEntry[]>("scan_universal_binaries");
      _cache.binaries = data;
      setEntries(data);
    } catch {
      setEntries([]);
    }
    setScanning(false);
  }, []);

  useEffect(() => {
    if (!_cache.binaries) load();
  }, [load]);

  const compatibleEntries = (entries ?? []).filter((entry) => !entry.thinning_unsafe);
  const totalReclaimable = compatibleEntries.reduce((s, e) => s + e.reclaimable_bytes, 0);

  const thin = async (entry: UniversalBinaryEntry) => {
    if (entry.thinning_unsafe) {
      onToast(false, entry.thinning_warning);
      return;
    }
    setThinningPath(entry.path);
    setConfirmPath(null);
    try {
      const result = await invoke<{ bytes_saved: number; binary_count: number }>(
        "thin_universal_app",
        { name: entry.name, appPath: entry.path }
      );
      onToast(
        true,
        `${entry.name} allégé : ${fmtBytes(result.bytes_saved)} récupérés · original dans la Corbeille`
      );
      await load();
    } catch (error) {
      onToast(false, String(error));
    } finally {
      setThinningPath(null);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between px-1">
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {scanning
            ? "Analyse…"
            : `${compatibleEntries.length} application${compatibleEntries.length !== 1 ? "s" : ""} compatible${compatibleEntries.length !== 1 ? "s" : ""}`}
          {totalReclaimable > 0 && (
            <span style={{ color: "var(--accent)" }}>
              {" "}
              · ~{fmtBytes(totalReclaimable)} récupérables
            </span>
          )}
        </span>
        <button
          onClick={load}
          disabled={scanning}
          className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={10} /> Actualiser
        </button>
      </div>

      {/* Informational note */}
      <div
        className="rounded-xl px-4 py-3 flex flex-col gap-2"
        style={{ background: "var(--bar-track)", border: "1px solid var(--border)" }}
      >
        <div className="flex items-start gap-2.5">
          <Cpu size={14} style={{ color: "var(--info)", flexShrink: 0, marginTop: 1 }} />
          <div className="text-[11px] font-medium" style={{ color: "var(--text-1)" }}>
            Binaires universels — qu'est-ce que c'est&nbsp;?
          </div>
        </div>
        <div className="text-[11px] leading-relaxed pl-[22px]" style={{ color: "var(--text-2)" }}>
          Les Mac Apple Silicon (M1, M2, M3…) peuvent faire tourner deux types de code : le code
          natif <strong>ARM</strong> (pour votre puce Apple) et le code{" "}
          <strong>Intel x86_64</strong>
          (hérité, exécuté via Rosetta 2). Les apps "universelles" embarquent les deux dans un seul
          fichier — ce qui prend deux fois plus de place. Sur un Mac Apple Silicon, la tranche Intel
          ne sert généralement pas sur cette machine. Burrow affiche ici l'espace correspondant à
          titre informatif.
        </div>
        <div className="flex items-start gap-2.5 pt-1">
          <AlertTriangle
            size={13}
            style={{ color: "var(--warning)", flexShrink: 0, marginTop: 1 }}
          />
          <div className="text-[11px] leading-relaxed" style={{ color: "var(--text-3)" }}>
            <strong style={{ color: "var(--warning)" }}>Opération récupérable</strong> — Burrow
            travaille sur une copie, préserve la signature de l'éditeur et vérifie son intégrité
            avant l'installation. Si la signature ne peut pas être conservée, l'opération est
            refusée et l'application originale reste intacte.
          </div>
        </div>
      </div>

      {scanning ? (
        <div
          className="flex items-center gap-2 py-6 justify-center"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={14} className="animate-spin" />
          <span className="text-xs">Scan des applications…</span>
        </div>
      ) : (entries?.length ?? 0) === 0 ? (
        <div className="text-center py-8 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun binaire universel trouvé
        </div>
      ) : (
        <div className="card overflow-hidden">
          {entries!.map((entry, i) => (
            <div
              key={entry.path}
              style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}
              className="flex items-center gap-3 px-4 py-3"
            >
              <Cpu size={15} style={{ color: "var(--info)", flexShrink: 0 }} />
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium truncate" style={{ color: "var(--text-1)" }}>
                  {entry.name}
                </div>
                <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                  {entry.path.replace(/^\/Applications\//, "")}
                </div>
                {entry.thinning_unsafe && (
                  <div className="text-[9px] mt-0.5" style={{ color: "var(--warning)" }}>
                    Non compatible — réinstallation ou mise à jour requise
                  </div>
                )}
              </div>
              <div className="text-right shrink-0">
                <div className="text-[10px] font-mono" style={{ color: "var(--text-2)" }}>
                  ~{fmtBytes(entry.reclaimable_bytes)}
                </div>
                <div className="text-[9px]" style={{ color: "var(--text-3)" }}>
                  {entry.binary_count} binaire{entry.binary_count > 1 ? "s" : ""}
                </div>
              </div>
              {confirmPath === entry.path ? (
                <div className="flex items-center gap-1.5 shrink-0">
                  <button
                    onClick={() => thin(entry)}
                    className="text-[10px] px-2 py-1 rounded-lg font-semibold"
                    style={{ background: "var(--warning)", color: "#1c1917" }}
                  >
                    Confirmer
                  </button>
                  <button
                    onClick={() => setConfirmPath(null)}
                    className="text-[10px] px-2 py-1 rounded-lg"
                    style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
                  >
                    Annuler
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => setConfirmPath(entry.path)}
                  disabled={thinningPath !== null || entry.thinning_unsafe}
                  title={entry.thinning_unsafe ? entry.thinning_warning : undefined}
                  className="text-[10px] px-2.5 py-1 rounded-lg font-semibold disabled:opacity-40 flex items-center gap-1"
                  style={{ background: "var(--accent)", color: "var(--on-accent)" }}
                >
                  {thinningPath === entry.path ? (
                    <Loader2 size={10} className="animate-spin" />
                  ) : (
                    <Cpu size={10} />
                  )}
                  Amincir
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Tab 8: AI Caches ─────────────────────────────────────────────────────────

function AiCachesTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [items, setItems] = useState<AiCacheItem[] | null>(_cache.aiCaches ?? null);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(!_cache.aiCaches);
  const [cleaning, setCleaning] = useState(false);

  const load = useCallback(async () => {
    setScanning(true);
    try {
      const data = await invoke<AiCacheItem[]>("scan_ai_caches");
      _cache.aiCaches = data;
      setItems(data);
    } catch {
      setItems([]);
    }
    setScanning(false);
  }, []);
  useEffect(() => {
    if (!_cache.aiCaches) load();
  }, [load]);

  const toggle = (id: string) =>
    setSel((prev) => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const handleClean = async () => {
    setCleaning(true);
    try {
      const freed = await invoke<number>("clean_ai_caches", { ids: Array.from(sel) });
      onToast(true, `${fmtBytes(freed)} libérés`);
      setItems((prev) => (prev ? prev.filter((i) => !sel.has(i.id)) : null));
      setSel(new Set());
    } catch (e) {
      onToast(false, String(e));
    }
    setCleaning(false);
  };

  const total = (items ?? []).filter((i) => sel.has(i.id)).reduce((s, i) => s + i.size_bytes, 0);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between px-1">
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {scanning
            ? "Analyse…"
            : `${items?.length ?? 0} élément(s) — ${fmtBytes((items ?? []).reduce((s, i) => s + i.size_bytes, 0))}`}
        </span>
        <button
          onClick={load}
          disabled={scanning}
          className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={10} /> Actualiser
        </button>
      </div>
      {scanning ? (
        <div
          className="flex items-center gap-2 py-6 justify-center"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={14} className="animate-spin" />
          <span className="text-xs">Analyse des caches AI…</span>
        </div>
      ) : (items?.length ?? 0) === 0 ? (
        <div className="text-center py-8 text-sm" style={{ color: "var(--text-3)" }}>
          Aucun cache AI trouvé
        </div>
      ) : (
        <div className="card overflow-hidden">
          {(items ?? []).map((item, i) => (
            <button
              key={item.id}
              onClick={() => toggle(item.id)}
              className="w-full flex items-center gap-3 px-4 py-3 text-left transition-colors"
              style={{
                borderTop: i > 0 ? "1px solid var(--border)" : undefined,
                borderLeft: sel.has(item.id) ? "2px solid var(--accent)" : "2px solid transparent",
              }}
            >
              <Bot size={14} style={{ color: "var(--accent)", flexShrink: 0 }} />
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                  {item.label}
                </div>
                <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                  {item.path.replace(/^\/Users\/[^/]+/, "~")}
                </div>
              </div>
              <span className="text-[11px] font-mono shrink-0" style={{ color: "var(--text-2)" }}>
                {fmtBytes(item.size_bytes)}
              </span>
            </button>
          ))}
        </div>
      )}
      {sel.size > 0 && (
        <button
          onClick={handleClean}
          disabled={cleaning}
          className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
        >
          {cleaning ? (
            <>
              <Loader2 size={14} className="animate-spin" />
              Nettoyage…
            </>
          ) : (
            <>
              <Trash2 size={14} />
              Nettoyer ({fmtBytes(total)})
            </>
          )}
        </button>
      )}
    </div>
  );
}

// ── Tab 9: Système (Purgeable + TM Snapshots + Simulators + Dev Caches) ───────

function SectionHeader({
  icon: Icon,
  title,
  color = "var(--accent)",
}: {
  icon: React.ElementType;
  title: string;
  color?: string;
}) {
  return (
    <div className="flex items-center gap-2 px-1 mt-1">
      <Icon size={13} style={{ color }} />
      <span className="text-[11px] font-semibold" style={{ color: "var(--text-2)" }}>
        {title}
      </span>
    </div>
  );
}

function SystemeTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [purgeable, setPurgeable] = useState<PurgeableInfo | null>(_cache.purgeable ?? null);
  const [snapshots, setSnapshots] = useState<TmSnapshot[] | null>(_cache.snapshots ?? null);
  const [sims, setSims] = useState<SimRuntime[] | null>(_cache.sims ?? null);
  const [devCaches, setDevCaches] = useState<DevCacheItem[] | null>(_cache.sysDevCaches ?? null);
  const [selDev, setSelDev] = useState<Set<string>>(new Set());
  const [cleaningDev, setCleaningDev] = useState(false);
  const [loading, setLoading] = useState(!_cache.purgeable && !_cache.snapshots);

  const load = useCallback(async () => {
    setLoading(true);
    const [p, snaps, simList, devs] = await Promise.allSettled([
      invoke<PurgeableInfo>("get_purgeable_space"),
      invoke<TmSnapshot[]>("scan_tm_snapshots"),
      invoke<SimRuntime[]>("scan_simulator_runtimes"),
      invoke<DevCacheItem[]>("scan_dev_caches"),
    ]);
    if (p.status === "fulfilled") {
      _cache.purgeable = p.value;
      setPurgeable(p.value);
    }
    if (snaps.status === "fulfilled") {
      _cache.snapshots = snaps.value;
      setSnapshots(snaps.value);
    }
    if (simList.status === "fulfilled") {
      _cache.sims = simList.value;
      setSims(simList.value);
    }
    if (devs.status === "fulfilled") {
      _cache.sysDevCaches = devs.value;
      setDevCaches(devs.value);
    }
    setLoading(false);
  }, []);
  useEffect(() => {
    if (!_cache.purgeable && !_cache.snapshots) load();
  }, [load]);

  const toggleDev = (id: string) =>
    setSelDev((prev) => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const handleCleanDev = async () => {
    setCleaningDev(true);
    try {
      const freed = await invoke<number>("clean_dev_caches", { ids: Array.from(selDev) });
      onToast(true, `${fmtBytes(freed)} libérés`);
      setDevCaches((prev) => (prev ? prev.filter((d) => !selDev.has(d.id)) : null));
      setSelDev(new Set());
    } catch (e) {
      onToast(false, String(e));
    }
    setCleaningDev(false);
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between px-1">
        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
          {loading ? "Analyse…" : "Optimisations système"}
        </span>
        <button
          onClick={load}
          disabled={loading}
          className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
          style={{ color: "var(--text-3)" }}
        >
          <RefreshCw size={10} /> Actualiser
        </button>
      </div>

      {/* APFS Purgeable */}
      <SectionHeader icon={Database} title="Espace APFS purgeable" />
      <div className="card px-4 py-3 flex items-center gap-3">
        <Database size={16} style={{ color: "var(--accent)", flexShrink: 0 }} />
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
            Espace géré par macOS
          </div>
          <div className="text-[10px]" style={{ color: "var(--text-3)" }}>
            {purgeable
              ? `${fmtBytes(purgeable.purgeable_bytes)} purgeable · ${fmtBytes(purgeable.free_bytes)} libre`
              : "—"}
          </div>
        </div>
        <span
          className="text-[9px] px-2 py-1 rounded-full"
          style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
        >
          Informatif
        </span>
      </div>

      {/* Time Machine Snapshots */}
      {(snapshots?.length ?? 0) > 0 && (
        <>
          <SectionHeader
            icon={Database}
            title={`Snapshots Time Machine (${snapshots!.length})`}
            color="var(--warning)"
          />
          <div className="card overflow-hidden">
            {snapshots!.map((s, i) => (
              <div
                key={s.date}
                className="flex items-center gap-3 px-4 py-3"
                style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}
              >
                <Database size={13} style={{ color: "var(--warning)", flexShrink: 0 }} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {s.date}
                  </div>
                  {s.size_bytes > 0 && (
                    <div className="text-[10px]" style={{ color: "var(--text-3)" }}>
                      {fmtBytes(s.size_bytes)}
                    </div>
                  )}
                </div>
                <span className="text-[9px]" style={{ color: "var(--text-3)" }}>
                  Géré par Time Machine
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Xcode Simulator Runtimes */}
      {(sims?.length ?? 0) > 0 && (
        <>
          <SectionHeader
            icon={Smartphone}
            title={`Runtimes simulateur Xcode (${sims!.length})`}
            color="var(--text-2)"
          />
          <div className="card overflow-hidden">
            {sims!.map((s, i) => (
              <div
                key={s.identifier}
                className="flex items-center gap-3 px-4 py-3"
                style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}
              >
                <Smartphone size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {s.platform} {s.version}
                  </div>
                  <div className="text-[10px]" style={{ color: "var(--text-3)" }}>
                    {fmtBytes(s.size_bytes)} · build {s.build}
                  </div>
                </div>
                <span
                  className="text-[9px] px-1.5 py-0.5 rounded"
                  style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                >
                  {s.deletable ? "informatif" : "actif"}
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Dev Caches */}
      {(devCaches?.length ?? 0) > 0 && (
        <>
          <SectionHeader icon={Code2} title="Caches développeur" color="var(--text-2)" />
          <div className="card overflow-hidden">
            {devCaches!.map((d, i) => (
              <button
                key={d.id}
                onClick={() => toggleDev(d.id)}
                className="w-full flex items-center gap-3 px-4 py-3 text-left transition-colors"
                style={{
                  borderTop: i > 0 ? "1px solid var(--border)" : undefined,
                  borderLeft: selDev.has(d.id)
                    ? "2px solid var(--accent)"
                    : "2px solid transparent",
                }}
              >
                <Code2 size={13} style={{ color: "var(--text-3)", flexShrink: 0 }} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {d.label}
                  </div>
                  <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                    {d.path.replace(/^\/Users\/[^/]+/, "~")}
                  </div>
                </div>
                <span className="text-[11px] font-mono shrink-0" style={{ color: "var(--text-2)" }}>
                  {fmtBytes(d.size_bytes)}
                </span>
              </button>
            ))}
          </div>
          {selDev.size > 0 && (
            <button
              onClick={handleCleanDev}
              disabled={cleaningDev}
              className="btn-primary flex items-center justify-center gap-2 disabled:opacity-50"
            >
              {cleaningDev ? (
                <>
                  <Loader2 size={14} className="animate-spin" />
                  Nettoyage…
                </>
              ) : (
                <>
                  <Trash2 size={14} />
                  Nettoyer sélection
                </>
              )}
            </button>
          )}
        </>
      )}

      {!loading &&
        (snapshots?.length ?? 0) === 0 &&
        (sims?.length ?? 0) === 0 &&
        (devCaches?.length ?? 0) === 0 && (
          <div className="text-center py-8 text-sm" style={{ color: "var(--text-3)" }}>
            Aucun élément système à optimiser
          </div>
        )}
    </div>
  );
}

// ── Tab 7: Login Items + Deleted Users ────────────────────────────────────────

// Modal de confirmation pour suppression groupée
interface DeleteConfirm {
  item: LoginItemEntry;
  related: string[];
}

function LoginItemsSection({
  label,
  icon: Icon,
  color,
  items,
  renderItem,
}: {
  label: string;
  icon: React.ElementType;
  color: string;
  items: LoginItemEntry[];
  renderItem: (item: LoginItemEntry) => React.ReactNode;
}) {
  if (items.length === 0) return null;
  return (
    <div className="card overflow-hidden">
      <div
        className="flex items-center gap-2 px-4 py-2.5"
        style={{ borderBottom: "1px solid var(--border)", background: "var(--bg)" }}
      >
        <Icon size={13} style={{ color }} />
        <span className="text-[11px] font-semibold" style={{ color }}>
          {label}
        </span>
        <span
          className="text-[10px] px-1.5 py-0.5 rounded-full font-medium"
          style={{ background: color + "22", color }}
        >
          {items.length}
        </span>
      </div>
      {items.map((item, i) => (
        <div key={item.plist_path} style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}>
          {renderItem(item)}
        </div>
      ))}
    </div>
  );
}

function LoginItemsConfirmModal({
  data,
  onClose,
  onConfirm,
}: {
  data: DeleteConfirm;
  onClose: () => void;
  onConfirm: (deleteAll: boolean) => void;
}) {
  const hasRelated = data.related.length > 0;
  const shortName = (p: string) => p.split("/").pop()?.replace(".plist", "") ?? p;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onClose}
    >
      <div
        className="rounded-2xl p-5 shadow-2xl max-w-sm w-full mx-4 flex flex-col gap-4"
        style={{ background: "var(--card)", border: "1px solid var(--border)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div>
          <div className="text-sm font-semibold mb-1" style={{ color: "var(--text-1)" }}>
            Supprimer {data.item.name} ?
          </div>
          {hasRelated ? (
            <>
              <div className="text-[11px] mb-2" style={{ color: "var(--text-3)" }}>
                {data.related.length} composant{data.related.length > 1 ? "s" : ""} lié
                {data.related.length > 1 ? "s" : ""} trouvé{data.related.length > 1 ? "s" : ""} —
                tout déplacer dans la Corbeille évite que l'item réapparaisse.
              </div>
              <div
                className="rounded-lg p-2 flex flex-col gap-1"
                style={{ background: "var(--bg)", border: "1px solid var(--border)" }}
              >
                <div className="text-[10px] truncate" style={{ color: "var(--warning)" }}>
                  • {shortName(data.item.plist_path)}
                </div>
                {data.related.map((r) => (
                  <div key={r} className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                    + {shortName(r)}
                  </div>
                ))}
              </div>
            </>
          ) : (
            <div className="text-[11px]" style={{ color: "var(--text-3)" }}>
              Cet élément sera déplacé dans la Corbeille et pourra être restauré.
            </div>
          )}
        </div>
        <div className="flex gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded-xl text-xs font-medium transition-opacity hover:opacity-70"
            style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
          >
            Annuler
          </button>
          {hasRelated && (
            <button
              onClick={() => onConfirm(false)}
              className="flex-1 py-2 rounded-xl text-xs font-medium transition-opacity hover:opacity-70"
              style={{ background: "var(--danger)22", color: "var(--danger)" }}
            >
              Celui-ci
            </button>
          )}
          <button
            onClick={() => onConfirm(true)}
            className="flex-1 py-2 rounded-xl text-xs font-medium transition-opacity hover:opacity-70"
            style={{ background: "var(--danger)", color: "#fff" }}
          >
            {hasRelated ? "Tout déplacer" : "Déplacer"}
          </button>
        </div>
      </div>
    </div>
  );
}

function LoginItemsTab({ onToast }: { onToast: (ok: boolean, msg: string) => void }) {
  const [loginItems, setLoginItems] = useState<LoginItemEntry[] | null>(_cache.loginItems ?? null);
  const [deletedUsers, setDeletedUsers] = useState<DeletedUserEntry[] | null>(
    _cache.deletedUsers ?? null
  );
  const [scanning, setScanning] = useState(!_cache.loginItems);
  const [acting, setActing] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<DeleteConfirm | null>(null);

  const load = useCallback(async () => {
    setScanning(true);
    try {
      const [li, du] = await Promise.all([
        invoke<LoginItemEntry[]>("scan_login_items"),
        invoke<DeletedUserEntry[]>("scan_deleted_users"),
      ]);
      _cache.loginItems = li;
      _cache.deletedUsers = du;
      setLoginItems(li);
      setDeletedUsers(du);
    } catch {
      setLoginItems([]);
      setDeletedUsers([]);
    }
    setScanning(false);
  }, []);

  useEffect(() => {
    if (!_cache.loginItems) load();
  }, [load]);

  const removePaths = (paths: string[]) => {
    const set = new Set(paths);
    setLoginItems((prev) => {
      const next = prev ? prev.filter((i) => !set.has(i.plist_path)) : null;
      _cache.loginItems = next;
      return next;
    });
  };

  // Clic sur corbeille → cherche les composants liés, puis affiche la confirmation
  const handleDeleteClick = async (item: LoginItemEntry) => {
    setActing(item.plist_path);
    try {
      const related = await invoke<string[]>("find_related_launch_items", {
        plistPath: item.plist_path,
      });
      setConfirm({ item, related });
    } catch {
      setConfirm({ item, related: [] });
    }
    setActing(null);
  };

  // Suppression effective après confirmation
  const handleDeleteConfirm = async (deleteAll: boolean) => {
    if (!confirm) return;
    const paths = deleteAll
      ? [confirm.item.plist_path, ...confirm.related]
      : [confirm.item.plist_path];
    setConfirm(null);
    setActing(confirm.item.plist_path);
    try {
      await invoke("delete_launch_items", { plistPaths: paths });
      removePaths(paths);
      onToast(
        true,
        `${paths.length} élément${paths.length > 1 ? "s" : ""} déplacé${paths.length > 1 ? "s" : ""} dans la Corbeille`
      );
    } catch (e) {
      onToast(false, String(e));
    }
    setActing(null);
  };

  const handleToggle = async (item: LoginItemEntry, enable: boolean) => {
    setActing(item.plist_path);
    try {
      await invoke("toggle_login_item", { plistPath: item.plist_path, enable });
      onToast(true, `${item.name} ${enable ? "activé" : "désactivé"}`);
    } catch (e) {
      onToast(false, String(e));
    }
    setActing(null);
  };

  const broken = (loginItems ?? []).filter((i) => i.is_broken);
  const suspicious = (loginItems ?? []).filter((i) => i.is_suspicious && !i.is_broken);
  const normal = (loginItems ?? []).filter((i) => !i.is_broken && !i.is_suspicious);

  function LoginRow({ item }: { item: LoginItemEntry }) {
    const isActing = acting === item.plist_path;
    const color = item.is_suspicious
      ? "var(--danger)"
      : item.is_broken
        ? "var(--warning)"
        : "var(--text-3)";
    const badge = item.is_suspicious ? "Suspect" : item.is_broken ? "Cassé" : "Normal";
    const badgeBg = item.is_suspicious
      ? "var(--danger)"
      : item.is_broken
        ? "var(--warning)"
        : "var(--bar-track)";
    const badgeColor = item.is_suspicious || item.is_broken ? "#fff" : "var(--text-3)";
    return (
      <div className="flex items-start gap-3 px-4 py-3">
        <Lock size={14} style={{ color, flexShrink: 0, marginTop: 3 }} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
              {item.name}
            </span>
            <span
              className="text-[9px] px-1.5 py-0.5 rounded font-medium shrink-0"
              style={{ background: badgeBg, color: badgeColor }}
            >
              {badge}
            </span>
            {item.is_system && (
              <span
                className="text-[9px] px-1.5 py-0.5 rounded font-medium shrink-0"
                style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
              >
                système
              </span>
            )}
          </div>
          <div className="text-[10px] truncate mt-0.5" style={{ color: "var(--text-3)" }}>
            {item.program || item.plist_path.replace(/^\/Users\/[^/]+/, "~")}
          </div>
          {item.suspicious_reason && (
            <div className="text-[10px] mt-0.5" style={{ color: "var(--danger)" }}>
              ⚠ {item.suspicious_reason}
            </div>
          )}
        </div>
        {/* Actions */}
        <div className="flex items-center gap-1.5 shrink-0">
          {isActing ? (
            <Loader2 size={13} className="animate-spin" style={{ color: "var(--accent)" }} />
          ) : (
            <>
              {!item.is_broken && (
                <button
                  onClick={() => handleToggle(item, false)}
                  title="Désactiver"
                  className="p-1.5 rounded transition-opacity hover:opacity-70"
                  style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                >
                  <svg
                    width="11"
                    height="11"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <path d="M18.36 6.64A9 9 0 1 1 5.64 19.36" />
                    <line x1="12" y1="2" x2="12" y2="12" />
                  </svg>
                </button>
              )}
              <button
                onClick={() => handleDeleteClick(item)}
                title="Supprimer"
                className="p-1.5 rounded transition-opacity hover:opacity-70"
                style={{
                  background: item.is_broken ? "var(--warning)22" : "var(--danger)18",
                  color: item.is_broken ? "var(--warning)" : "var(--danger)",
                }}
              >
                <Trash2 size={11} />
              </button>
            </>
          )}
        </div>
      </div>
    );
  }

  return (
    <>
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between px-1">
          <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
            {scanning
              ? "Analyse…"
              : `${loginItems?.length ?? 0} login item${(loginItems?.length ?? 0) !== 1 ? "s" : ""}`}
          </span>
          <button
            onClick={load}
            disabled={scanning}
            className="text-[11px] flex items-center gap-1 opacity-60 hover:opacity-100"
            style={{ color: "var(--text-3)" }}
          >
            <RefreshCw size={10} /> Actualiser
          </button>
        </div>

        {scanning ? (
          <div
            className="flex items-center gap-2 py-6 justify-center"
            style={{ color: "var(--text-3)" }}
          >
            <Loader2 size={14} className="animate-spin" />
            <span className="text-xs">Analyse des agents de démarrage…</span>
          </div>
        ) : (
          <>
            <LoginItemsSection
              label="Suspects"
              icon={AlertTriangle}
              color="var(--danger)"
              items={suspicious}
              renderItem={(item) => <LoginRow item={item} />}
            />
            <LoginItemsSection
              label="Cassés"
              icon={UserX}
              color="var(--warning)"
              items={broken}
              renderItem={(item) => <LoginRow item={item} />}
            />
            <LoginItemsSection
              label="Normaux"
              icon={Users}
              color="var(--text-3)"
              items={normal}
              renderItem={(item) => <LoginRow item={item} />}
            />
            {loginItems?.length === 0 && (
              <div className="text-center py-8 text-sm" style={{ color: "var(--text-3)" }}>
                Aucun login item trouvé
              </div>
            )}

            {(deletedUsers?.length ?? 0) > 0 && (
              <>
                <div
                  className="text-[11px] font-semibold px-1 mt-2"
                  style={{ color: "var(--text-2)" }}
                >
                  Dossiers home orphelins
                </div>
                <div className="card overflow-hidden">
                  {deletedUsers!.map((u, i) => (
                    <div
                      key={u.home_path}
                      className="flex items-center gap-3 px-4 py-3"
                      style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}
                    >
                      <UserX size={14} style={{ color: "var(--warning)", flexShrink: 0 }} />
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                          {u.username}
                        </div>
                        <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                          {u.home_path}
                        </div>
                      </div>
                      <span
                        className="text-[11px] font-mono shrink-0 px-2 py-0.5 rounded"
                        style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
                      >
                        {fmtBytes(u.size_bytes)}
                      </span>
                    </div>
                  ))}
                </div>
              </>
            )}
          </>
        )}
      </div>
      {confirm && (
        <LoginItemsConfirmModal
          data={confirm}
          onClose={() => setConfirm(null)}
          onConfirm={handleDeleteConfirm}
        />
      )}
    </>
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────

export default function Clean() {
  const [tab, setTab] = useState(_cache.currentTab ?? 0);
  const [mounted, setMounted] = useState<Set<number>>(new Set(_cache.mountedTabs ?? [0]));
  const [caches, setCaches] = useState<DevCache[] | null>(_cache.caches ?? null);
  const [cachesLoading, setCachesLoading] = useState(!_cache.cachesLoaded);
  const [selectSafeTrigger, setSafeTrigger] = useState(0);
  const [toast, setToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const [loadingItems, setLoadingItems] = useState([
    { id: "caches", label: "Analyse des caches développeur", done: false },
    { id: "storage", label: "Calcul des tailles de stockage", done: false },
  ]);

  const showToast = useCallback((ok: boolean, msg: string) => {
    setToast({ ok, msg });
    setTimeout(() => setToast(null), 3500);
  }, []);

  const loadCaches = useCallback(async () => {
    setCachesLoading(true);
    setLoadingItems([
      { id: "caches", label: "Analyse des caches développeur", done: false },
      { id: "storage", label: "Calcul des tailles de stockage", done: false },
    ]);
    try {
      const data = await invoke<DevCache[]>("get_dev_caches");
      _cache.caches = data;
      _cache.cachesLoaded = true;
      setCaches(data);
      setLoadingItems((p) => p.map((i) => ({ ...i, done: true })));
    } catch (e) {
      console.error("get_dev_caches:", e);
    }
    setCachesLoading(false);
  }, []);

  useEffect(() => {
    if (!_cache.cachesLoaded) {
      const t = setTimeout(() => loadCaches(), 200);
      return () => clearTimeout(t);
    }
  }, [loadCaches]);

  const switchTab = (idx: number) => {
    _cache.currentTab = idx;
    setTab(idx);
    setMounted((prev) => {
      const next = new Set([...prev, idx]);
      _cache.mountedTabs = [...next];
      return next;
    });
  };

  const handleCleanSafe = () => {
    setSafeTrigger((p) => p + 1);
    switchTab(2);
  };

  if (cachesLoading && caches === null) {
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
    <div className="flex flex-col h-full px-6 pb-4 gap-4">
      {/* Header */}
      <div className="flex items-center gap-3 pt-1">
        <div
          className="w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <Trash2 size={16} style={{ color: "var(--text-3)" }} />
        </div>
        <div>
          <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
            Nettoyer
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            Nettoyage et analyse du stockage
          </p>
        </div>
      </div>

      {/* Tab bar */}
      <div
        className="flex gap-1 p-1 rounded-xl shrink-0"
        style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
      >
        {TABS.map(({ label, icon: Icon }, idx) => (
          <button
            key={idx}
            onClick={() => switchTab(idx)}
            className="flex-1 flex items-center justify-center gap-1 py-1.5 px-0.5 rounded-lg text-[10px] font-medium transition-all"
            style={{
              background: tab === idx ? "var(--bg)" : "transparent",
              color: tab === idx ? "var(--text-1)" : "var(--text-3)",
              boxShadow: tab === idx ? "0 1px 3px rgba(0,0,0,0.1)" : "none",
            }}
          >
            <Icon size={11} />
            <span>{label}</span>
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-y-auto min-h-0">
        <div style={{ display: tab === 0 ? "block" : "none" }}>
          {mounted.has(0) && (
            <ApercuTab
              caches={caches}
              cachesLoading={cachesLoading}
              onCleanSafe={handleCleanSafe}
              onOpenTab={switchTab}
            />
          )}
        </div>
        <div className="relative" style={{ display: tab === 1 ? "block" : "none" }}>
          {mounted.has(1) && <NettoyerTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 2 ? "block" : "none" }}>
          {mounted.has(2) && (
            <DeveloppeurTab
              caches={caches}
              cachesLoading={cachesLoading}
              onReload={loadCaches}
              selectSafeTrigger={selectSafeTrigger}
              onToast={showToast}
            />
          )}
        </div>
        <div style={{ display: tab === 3 ? "block" : "none" }}>
          {mounted.has(3) && <FichiersTab onToast={showToast} />}
        </div>
        <div className="flex flex-col h-full" style={{ display: tab === 4 ? "flex" : "none" }}>
          {mounted.has(4) && <DoublonsTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 5 ? "block" : "none" }}>
          {mounted.has(5) && <PrivacyTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 6 ? "block" : "none" }}>
          {mounted.has(6) && <BinairesTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 7 ? "block" : "none" }}>
          {mounted.has(7) && <LoginItemsTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 8 ? "block" : "none" }}>
          {mounted.has(8) && <AiCachesTab onToast={showToast} />}
        </div>
        <div style={{ display: tab === 9 ? "block" : "none" }}>
          {mounted.has(9) && <SystemeTab onToast={showToast} />}
        </div>
      </div>

      {/* Toast notification bottom-right */}
      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.95 }}
            transition={{ duration: 0.18 }}
            className="fixed bottom-4 right-4 z-50 flex items-center gap-2 px-4 py-2.5 rounded-xl text-[12px] font-medium shadow-lg"
            style={{
              background: toast.ok ? "var(--success)" : "var(--danger)",
              color: "#fff",
              boxShadow: `0 4px 16px ${toast.ok ? "var(--success-soft)" : "var(--danger-soft)"}`,
            }}
          >
            {toast.ok ? <Check size={13} /> : <AlertCircle size={13} />}
            {toast.msg}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
