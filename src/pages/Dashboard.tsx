import { useState, useEffect, useRef, useMemo, memo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Sparkles,
  Cpu,
  HardDrive,
  Server,
  Wifi,
  Battery,
  BatteryCharging,
  Bluetooth,
  Thermometer,
  ChevronUp,
  ChevronDown,
  ChevronsUpDown,
  MonitorDot,
  X,
  CheckCircle2,
  Loader2,
  MoreHorizontal,
  Check,
  AlertCircle,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useT } from "../i18n/useT";

interface NetInterface {
  name: string;
  ip: string;
  rx_rate: number;
  tx_rate: number;
}
interface NetRateItem {
  name: string;
  rx_bps: number;
  tx_bps: number;
}
interface GpuInfo {
  model: string;
  vram_mb: number;
  vendor: string;
}
interface ProcessEntry {
  pids: number[];
  name: string;
  cpu_usage: number;
  memory_bytes: number;
  disk_read_bytes: number;
  disk_written_bytes: number;
}
interface ProcessInfo {
  name: string;
  command: string;
  cpu: number;
  memory: number;
}
interface BluetoothDevice {
  name: string;
  connected: boolean;
  battery: string;
}

interface QuickMetrics {
  cpu_usage: number;
  cpu_per_core: number[];
  cpu_core_count: number;
  cpu_load1: number;
  cpu_load5: number;
  cpu_load15: number;
  mem_used: number;
  mem_total: number;
  mem_used_percent: number;
  mem_swap_used: number;
  mem_swap_total: number;
  disk_used: number;
  disk_total: number;
  disk_used_percent: number;
  uptime_secs: number;
  gpu_busy_percent: number;
  fan_speed_rpm: number;
  // Temperatures (°C) — IOHID, no root
  cpu_temp: number;
  gpu_temp: number;
  soc_temp: number;
  nand_temp: number;
  ane_temp: number;
  // Power (W) — IOReport Energy Model, no root
  cpu_power: number;
  gpu_power: number;
  ram_power: number;
  ane_power: number;
}

interface SystemMetrics {
  host: string;
  platform: string;
  uptime: string;
  procs: number;
  model: string;
  cpu_model: string;
  os_version: string;
  health_score: number;
  health_score_msg: string;
  cpu_usage: number;
  cpu_per_core: number[];
  cpu_load1: number;
  cpu_load5: number;
  cpu_load15: number;
  cpu_core_count: number;
  mem_used: number;
  mem_total: number;
  mem_available: number;
  mem_used_percent: number;
  mem_swap_used: number;
  mem_swap_total: number;
  disk_used: number;
  disk_total: number;
  disk_used_percent: number;
  disk_io_read: number;
  disk_io_write: number;
  trash_size: number;
  net_interfaces: NetInterface[];
  proxy_enabled: boolean;
  proxy_type: string;
  proxy_host: string;
  battery_percent: number;
  battery_status: string;
  battery_time_left: string;
  battery_health: string;
  battery_cycles: number;
  battery_capacity: number;
  thermal_cpu_temp: number;
  thermal_battery_temp: number;
  thermal_system_power: number;
  thermal_adapter_power: number;
  thermal_battery_power: number;
  thermal_fan_speed: number;
  top_processes: ProcessInfo[];
  bluetooth_devices: BluetoothDevice[];
}

function fmtBytes(b: number) {
  if (b >= 1e12) return `${(b / 1e12).toFixed(2)} TB`;
  if (b >= 1e9) return `${(b / 1e9).toFixed(2)} GB`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} MB`;
  return `${(b / 1e3).toFixed(0)} KB`;
}
function fmtBytesGo(b: number) {
  if (b >= 1e12) return `${(b / 1e12).toFixed(2)} To`;
  if (b >= 1e9) return `${(b / 1e9).toFixed(2)} Go`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} Mo`;
  return `${(b / 1e3).toFixed(0)} Ko`;
}
function fmtMbs(r: number) {
  if (r === 0) return "0 B/s";
  if (r >= 1) return `${r.toFixed(2)} MB/s`;
  return `${(r * 1024).toFixed(0)} KB/s`;
}
function fmtBps(bps: number) {
  if (bps >= 1_000_000) return `${(bps / 1e6).toFixed(1)} MB/s`;
  if (bps >= 1_000) return `${(bps / 1000).toFixed(0)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

// Translate Mole's English health_score_msg into the current UI language
const HEALTH_MSG_MAP: Record<string, Record<string, string>> = {
  fr: {
    Excellent: "Excellent",
    "Very Good": "Très bien",
    Good: "Bon",
    Fair: "Passable",
    Poor: "Mauvais",
    Critical: "Critique",
    "High Memory": "Mémoire élevée",
    "High CPU": "CPU élevé",
    "High CPU Usage": "CPU élevé",
    "High Disk Usage": "Disque saturé",
    "Low Battery": "Batterie faible",
    "Restart Recommended": "Redémarrage recommandé",
    "No issues found": "Aucun problème détecté",
    "Minor issues": "Problèmes mineurs",
    "Multiple Issues Detected": "Plusieurs problèmes",
    "Multiple Issues": "Plusieurs problèmes",
    "Issues Detected": "Problèmes détectés",
  },
  es: {
    Excellent: "Excelente",
    "Very Good": "Muy bien",
    Good: "Bien",
    Fair: "Regular",
    Poor: "Malo",
    "High Memory": "Memoria alta",
    "High CPU Usage": "CPU alto",
    "High Disk Usage": "Disco saturado",
    "Low Battery": "Batería baja",
    "Restart Recommended": "Reinicio recomendado",
  },
  de: {
    Excellent: "Ausgezeichnet",
    "Very Good": "Sehr gut",
    Good: "Gut",
    Fair: "Mäßig",
    Poor: "Schlecht",
    "High Memory": "Hoher Speicher",
    "High CPU Usage": "Hohe CPU-Nutzung",
    "Low Battery": "Niedriger Akku",
    "Restart Recommended": "Neustart empfohlen",
  },
  zh: {
    Excellent: "优秀",
    "Very Good": "很好",
    Good: "良好",
    Fair: "一般",
    Poor: "差",
    "High Memory": "内存占用高",
    "High CPU Usage": "CPU占用高",
    "Low Battery": "电量低",
    "Restart Recommended": "建议重启",
  },
};

function translateHealthMsg(msg: string, lang: string): string {
  if (!msg || lang === "en") return msg;
  const map = HEALTH_MSG_MAP[lang] ?? {};
  let result = msg;
  for (const [eng, loc] of Object.entries(map)) {
    result = result.replace(new RegExp(eng, "gi"), loc);
  }
  return result;
}

// History persists across tab mount/unmount — shared module-level store
const _hist = {
  mem: [] as number[],
  disk: [] as number[],
  rx: [] as number[],
  tx: [] as number[],
  gpu: [] as number[],
};
const HISTORY_MAX = 30; // 30 × 2 s = 60 s

// ── Smooth bezier sparkline ──────────────────────────────────────────────────

function Sparkline({
  data,
  color,
  height = 36,
}: {
  data: number[];
  color: string;
  height?: number;
}) {
  if (data.length < 2) return <div style={{ height }} />;
  const max = Math.max(...data, 0.001);
  const W = 200;
  const H = height;
  const pad = 2;

  const pts = data.map((v, i) => [
    (i / (data.length - 1)) * W,
    H - pad - (v / max) * (H - pad * 2),
  ]);

  // Cubic bezier smooth path
  let linePath = `M ${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
  for (let i = 1; i < pts.length; i++) {
    const cpx = (pts[i - 1][0] + pts[i][0]) / 2;
    linePath += ` C ${cpx.toFixed(1)},${pts[i - 1][1].toFixed(1)} ${cpx.toFixed(1)},${pts[i][1].toFixed(1)} ${pts[i][0].toFixed(1)},${pts[i][1].toFixed(1)}`;
  }
  const areaPath = `${linePath} L ${pts[pts.length - 1][0]},${H} L ${pts[0][0]},${H} Z`;
  const uid = `sg-${color.replace(/[^a-z0-9]/gi, "")}`;

  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: "100%", height }} preserveAspectRatio="none">
      <defs>
        <linearGradient id={uid} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.28" />
          <stop offset="100%" stopColor={color} stopOpacity="0.02" />
        </linearGradient>
      </defs>
      <path d={areaPath} fill={`url(#${uid})`} />
      <path
        d={linePath}
        fill="none"
        stroke={color}
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// ── Bidirectional network sparkline (download ↑, upload ↓, no gap) ──────────

function BiDirSparkline({ rx, tx, height = 44 }: { rx: number[]; tx: number[]; height?: number }) {
  const W = 200;
  const H = height;
  const half = H / 2;
  const pad = 1;

  const maxVal = Math.max(...rx, ...tx, 0.001);

  function makePath(data: number[], flip: boolean) {
    if (data.length < 2) return { line: "", area: "" };
    const pts = data.map((v, i) => {
      const x = (i / (data.length - 1)) * W;
      const ratio = v / maxVal;
      const y = flip
        ? half + pad + ratio * (half - pad) // upload grows downward from center
        : half - pad - ratio * (half - pad); // download grows upward from center
      return [x, y];
    });
    let line = `M ${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
    for (let i = 1; i < pts.length; i++) {
      const cpx = (pts[i - 1][0] + pts[i][0]) / 2;
      line += ` C ${cpx.toFixed(1)},${pts[i - 1][1].toFixed(1)} ${cpx.toFixed(1)},${pts[i][1].toFixed(1)} ${pts[i][0].toFixed(1)},${pts[i][1].toFixed(1)}`;
    }
    const area = `${line} L ${pts[pts.length - 1][0]},${half} L ${pts[0][0]},${half} Z`;
    return { line, area };
  }

  const rxPath = makePath(rx, false);
  const txPath = makePath(tx, true);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: "100%", height }} preserveAspectRatio="none">
      <defs>
        <linearGradient id="bi-rx" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--cyan)" stopOpacity="0.35" />
          <stop offset="50%" stopColor="var(--cyan)" stopOpacity="0.05" />
        </linearGradient>
        <linearGradient id="bi-tx" x1="0" y1="1" x2="0" y2="0">
          <stop offset="0%" stopColor="var(--violet)" stopOpacity="0.35" />
          <stop offset="50%" stopColor="var(--violet)" stopOpacity="0.05" />
        </linearGradient>
      </defs>
      {/* center axis */}
      <line x1="0" y1={half} x2={W} y2={half} stroke="var(--border)" strokeWidth="0.5" />
      {/* download (up) */}
      <path d={rxPath.area} fill="url(#bi-rx)" />
      <path
        d={rxPath.line}
        fill="none"
        stroke="var(--cyan)"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* upload (down) */}
      <path d={txPath.area} fill="url(#bi-tx)" />
      <path
        d={txPath.line}
        fill="none"
        stroke="var(--violet)"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// ── Per-core CPU visualization ───────────────────────────────────────────────

function CoreBlocks({ perCore }: { perCore: number[] }) {
  if (perCore.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-0.5 mt-2">
      {perCore.map((pct, i) => {
        const color =
          pct > 80 ? "var(--danger-text)" : pct > 50 ? "var(--warning)" : "var(--success)";
        return (
          <div
            key={i}
            className="relative rounded-sm overflow-hidden"
            title={`Core ${i + 1}: ${pct.toFixed(0)}%`}
            style={{
              width: perCore.length > 8 ? 13 : 18,
              height: 5,
              background: "var(--bar-track)",
            }}
          >
            <div
              className="absolute inset-y-0 left-0"
              style={{ width: `${pct}%`, background: color }}
            />
          </div>
        );
      })}
    </div>
  );
}

// ── Other shared components ──────────────────────────────────────────────────

// Hex bruts obligatoires : concaténés avec un alpha ("…25") plus bas
const AVATAR_COLORS = [
  "#6366f1",
  "#8b6cc9",
  "#d6537e",
  "#dd8f1e",
  "#10b981",
  "#4a90d9",
  "#e05545",
  "#14b8a6",
];

function ProcAvatar({ name }: { name: string }) {
  const color = AVATAR_COLORS[(name.charCodeAt(0) || 0) % AVATAR_COLORS.length];
  return (
    <div
      className="w-5 h-5 rounded-md flex items-center justify-center text-[9px] font-bold shrink-0"
      style={{ background: color + "25", color }}
    >
      {(name[0] ?? "?").toUpperCase()}
    </div>
  );
}

function ProcessIcon({ name, icons }: { name: string; icons: Record<string, string> }) {
  const src = icons[name];
  if (src) return <img src={src} className="w-5 h-5 rounded-md object-cover shrink-0" alt={name} />;
  return <ProcAvatar name={name} />;
}

function MetricBar({ pct, color }: { pct: number; color?: string }) {
  return (
    <div className="h-1 rounded-full w-full" style={{ background: "var(--bar-track)" }}>
      <div
        className="h-1 rounded-full transition-all duration-500"
        style={{ width: `${Math.min(pct, 100)}%`, background: color ?? "var(--accent)" }}
      />
    </div>
  );
}

function FillBar({
  pct,
  used,
  total,
  color,
}: {
  pct: number;
  used: number;
  total: number;
  color: string;
}) {
  return (
    <div className="mt-2">
      <div
        className="h-2 rounded-full w-full overflow-hidden"
        style={{ background: "var(--bar-track)" }}
      >
        <div
          className="h-2 rounded-full transition-all duration-500"
          style={{ width: `${Math.min(pct, 100)}%`, background: color }}
        />
      </div>
      <div className="flex justify-between text-[10px] mt-1" style={{ color: "var(--text-3)" }}>
        <span>{fmtBytesGo(used)} utilisé</span>
        <span>
          {fmtBytesGo(total)} total · {pct.toFixed(0)}%
        </span>
      </div>
    </div>
  );
}

function CardHeader({
  icon: Icon,
  label,
  badge,
  color,
  children,
}: {
  icon: React.ElementType;
  label: string;
  badge?: string;
  color: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between mb-2.5">
      <div className="flex items-center gap-1.5">
        <Icon size={11} style={{ color }} />
        <span className="text-[10px] font-bold uppercase tracking-widest" style={{ color }}>
          {label}
        </span>
      </div>
      <div className="flex items-center gap-1.5">
        {children}
        {badge && (
          <span className="text-[10px] font-medium" style={{ color }}>
            {badge}
          </span>
        )}
      </div>
    </div>
  );
}

function BigNum({ value, unit, sub }: { value: string; unit?: string; sub?: string }) {
  return (
    <div className="flex items-baseline gap-1">
      <span
        className="text-4xl font-bold tracking-tight leading-none"
        style={{ color: "var(--text-1)" }}
      >
        {value}
      </span>
      {unit && (
        <span className="text-sm font-medium" style={{ color: "var(--text-3)" }}>
          {unit}
        </span>
      )}
      {sub && (
        <span className="text-sm ml-0.5" style={{ color: "var(--text-3)" }}>
          {sub}
        </span>
      )}
    </div>
  );
}

function Detail({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[11px] leading-relaxed" style={{ color: "var(--text-3)" }}>
      {children}
    </div>
  );
}

type SortCol = "cpu" | "mem" | "name";

function SortIcon({ col, active, dir }: { col: SortCol; active: SortCol; dir: "asc" | "desc" }) {
  if (col !== active)
    return <ChevronsUpDown size={10} style={{ color: "var(--text-3)", opacity: 0.5 }} />;
  return dir === "desc" ? (
    <ChevronDown size={10} style={{ color: "var(--accent)" }} />
  ) : (
    <ChevronUp size={10} style={{ color: "var(--accent)" }} />
  );
}

function cpuColor(pct: number) {
  return pct > 80 ? "var(--danger-text)" : pct > 50 ? "var(--warning)" : "var(--success)";
}

const ROW_H = 30;

const ProcessRow = memo(function ProcessRow({
  p,
  icons,
  onKill,
  flashing,
}: {
  p: ProcessEntry;
  icons: Record<string, string>;
  onKill: (p: ProcessEntry) => void;
  flashing: boolean;
}) {
  return (
    <tr
      className="tr-hover group"
      style={{
        borderBottom: "1px solid var(--border)",
        height: ROW_H,
        background: flashing ? "rgba(99,102,241,0.12)" : undefined,
        transition: "background 0.6s ease",
      }}
    >
      <td className="py-1.5 px-4">
        <div className="flex items-center gap-2.5 min-w-0">
          <ProcessIcon name={p.name} icons={icons} />
          <span className="truncate" style={{ color: "var(--text-1)" }}>
            {p.name}
          </span>
          {p.pids.length > 1 && (
            <span
              className="shrink-0 text-[9px] font-medium px-1 rounded"
              style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
            >
              {p.pids.length}×
            </span>
          )}
        </div>
      </td>
      <td className="py-1.5 px-4 text-right w-40">
        <div className="flex items-center justify-end gap-2">
          <div
            className="w-16 h-1 rounded-full overflow-hidden"
            style={{ background: "var(--bar-track)" }}
          >
            <div
              className="h-1 rounded-full transition-all duration-500"
              style={{ width: `${Math.min(p.cpu_usage, 100)}%`, background: cpuColor(p.cpu_usage) }}
            />
          </div>
          <span
            className="font-mono w-10 text-right shrink-0 text-[11px]"
            style={{ color: "var(--text-2)" }}
          >
            {p.cpu_usage.toFixed(1)}%
          </span>
        </div>
      </td>
      <td
        className="py-1.5 px-4 text-right w-24 font-mono text-[11px]"
        style={{ color: "var(--text-2)" }}
      >
        {p.memory_bytes >= 1e9
          ? `${(p.memory_bytes / 1e9).toFixed(1)} GB`
          : `${(p.memory_bytes / 1e6).toFixed(0)} MB`}
      </td>
      <td className="py-1.5 px-4 w-10 text-right">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onKill(p);
          }}
          className="rounded p-0.5 opacity-0 group-hover:opacity-100 transition-opacity"
          style={{ color: "var(--text-3)" }}
          title="Terminer le processus"
        >
          <MoreHorizontal size={13} />
        </button>
      </td>
    </tr>
  );
});

interface DashboardProps {
  onNavigate?: (page: string) => void;
}

export default function Dashboard({ onNavigate }: DashboardProps) {
  const { t, lang } = useT();
  const [qm, setQm] = useState<QuickMetrics | null>(null);
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sortCol, setSortCol] = useState<SortCol>("cpu");
  const [sortDir, setSortDir] = useState<"asc" | "desc">("desc");
  const [netRates, setNetRates] = useState<NetRateItem[]>([]);
  // Track selected network interface by name (stays stable across rate updates)
  const [selectedIface, setSelectedIface] = useState("en0");
  const switchIface = (iface: string) => {
    setSelectedIface(iface);
    _hist.rx = [];
    _hist.tx = [];
    setRxHistory([]);
    setTxHistory([]);
  };
  const [gpuInfo, setGpuInfo] = useState<GpuInfo | null>(null);
  const [processes, setProcesses] = useState<ProcessEntry[]>([]);
  const [killTarget, setKillTarget] = useState<ProcessEntry | null>(null);
  const [procTab, setProcTab] = useState<"cpu" | "ram" | "all">("cpu");
  const [procSearch, setProcSearch] = useState("");
  const [flashKeys, setFlashKeys] = useState<Set<string>>(new Set());
  const prevValuesRef = useRef<Map<string, { cpu: number; mem: number }>>(new Map());
  const [lowPower, setLowPower] = useState(false);
  const [showMemModal, setShowMemModal] = useState(false);
  const [memFreeRunning, setMemFreeRunning] = useState(false);
  const [memFreeToast, setMemFreeToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const [appIcons, setAppIcons] = useState<Record<string, string>>({});
  const [vScrollTop, setVScrollTop] = useState(0);
  const [vListHeight, setVListHeight] = useState(300);
  const processListRef = useRef<HTMLDivElement>(null);
  const processesLoadedRef = useRef(false);

  // History buffers for sparklines (up to HISTORY_MAX samples = ~3 min at 3s)
  const [_memHistory, setMemHistory] = useState<number[]>(() => [..._hist.mem]);
  const [rxHistory, setRxHistory] = useState<number[]>(() => [..._hist.rx]);
  const [txHistory, setTxHistory] = useState<number[]>(() => [..._hist.tx]);
  const [gpuHistory, setGpuHistory] = useState<number[]>(() => [..._hist.gpu]);

  // Prevent periodic refresh from overriding an optimistic low-power toggle
  const lowPowerPending = useRef<boolean | null>(null);
  const selectedIfaceRef = useRef(selectedIface);
  useEffect(() => {
    selectedIfaceRef.current = selectedIface;
  }, [selectedIface]);

  const loadQuick = async () => {
    try {
      const q = await invoke<QuickMetrics>("get_quick_metrics");
      setQm(q);
      _hist.mem = [..._hist.mem.slice(-(HISTORY_MAX - 1)), q.mem_used_percent];
      _hist.gpu = [..._hist.gpu.slice(-(HISTORY_MAX - 1)), q.gpu_busy_percent];
      setMemHistory([..._hist.mem]);
      setGpuHistory([..._hist.gpu]);
    } catch {}
  };

  const load = async (showLoader = true) => {
    if (showLoader) {
      setLoading(true);
      setError(null);
    }
    try {
      const [m, lp] = await Promise.all([
        invoke<SystemMetrics>("get_system_metrics"),
        invoke<boolean>("get_low_power_mode"),
      ]);
      setMetrics(m);
      // Only sync low power state from backend when no local change is pending
      if (lowPowerPending.current === null) {
        setLowPower(lp);
      }
      if (showLoader) setLoading(false);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("loading")) {
        // Pre-warm still in progress — keep spinner
      } else {
        setError(msg);
        if (showLoader) setLoading(false);
      }
    }
  };

  const loadNetRates = async () => {
    try {
      const rates = await invoke<NetRateItem[]>("get_net_rates");
      setNetRates(rates);
      const iface = selectedIfaceRef.current;
      const rate = rates.find((r) => r.name === iface);
      _hist.rx = [..._hist.rx.slice(-(HISTORY_MAX - 1)), rate ? rate.rx_bps / 1e6 : 0];
      _hist.tx = [..._hist.tx.slice(-(HISTORY_MAX - 1)), rate ? rate.tx_bps / 1e6 : 0];
      setRxHistory([..._hist.rx]);
      setTxHistory([..._hist.tx]);
    } catch {}
  };

  useEffect(() => {
    const el = processListRef.current;
    if (!el) return;
    setVListHeight(el.clientHeight);
    const ro = new ResizeObserver(() => setVListHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const [loadingItems, setLoadingItems] = useState<{ id: string; label: string; done: boolean }[]>([
    { id: "quick", label: t.dash_load_quick, done: false },
    { id: "metrics", label: t.dash_load_metrics, done: false },
    { id: "processes", label: t.dash_load_processes, done: false },
    { id: "network", label: t.dash_load_network, done: false },
  ]);
  const markDone = (id: string) =>
    setLoadingItems((p) => p.map((i) => (i.id === id ? { ...i, done: true } : i)));

  useEffect(() => {
    loadQuick().then(() => markDone("quick"));
    load().then(() => markDone("metrics"));
    loadNetRates().then(() => markDone("network"));
    invoke<GpuInfo>("get_gpu_info")
      .then(setGpuInfo)
      .catch(() => {});
    invoke<Record<string, string>>("get_all_app_icons")
      .then(setAppIcons)
      .catch(() => {});

    const quickInterval = setInterval(() => loadQuick(), 2000);
    const metricsInterval = setInterval(() => load(false), 3000);
    const netInterval = setInterval(() => loadNetRates(), 2000);

    let unlistenProcs: (() => void) | null = null;
    listen<ProcessEntry[]>("emit_processes", ({ payload }) => {
      if (!processesLoadedRef.current) {
        processesLoadedRef.current = true;
        markDone("processes");
      }
      const changed = new Set<string>();
      for (const p of payload) {
        const prev = prevValuesRef.current.get(p.name);
        if (prev && Math.abs(p.cpu_usage - prev.cpu) > 3) changed.add(p.name);
        prevValuesRef.current.set(p.name, { cpu: p.cpu_usage, mem: p.memory_bytes });
      }
      setProcesses(payload);
      if (changed.size > 0) {
        setFlashKeys(changed);
        setTimeout(() => setFlashKeys(new Set()), 600);
      }
    }).then((fn) => {
      unlistenProcs = fn;
    });

    return () => {
      clearInterval(quickInterval);
      clearInterval(metricsInterval);
      clearInterval(netInterval);
      if (unlistenProcs) unlistenProcs();
    };
  }, []);

  const toggleSort = (col: SortCol) => {
    if (sortCol === col) setSortDir((d) => (d === "desc" ? "asc" : "desc"));
    else {
      setSortCol(col);
      setSortDir("desc");
    }
  };

  const free_memory_action = async () => {
    setMemFreeRunning(true);
    const unlisten1 = await listen<string>("mo-output", () => {});
    const unlisten2 = await listen<number>("mo-done", () => {
      setMemFreeRunning(false);
      unlisten1();
      unlisten2();
      setMemFreeToast({ ok: true, msg: "Mémoire libérée avec succès" });
      setTimeout(() => setMemFreeToast(null), 3500);
    });
    invoke("free_memory").catch(() => {
      setMemFreeRunning(false);
      unlisten1();
      unlisten2();
      setMemFreeToast({ ok: false, msg: "Erreur lors de la libération mémoire" });
      setTimeout(() => setMemFreeToast(null), 3500);
    });
  };

  const filtered = useMemo(() => {
    if (!procSearch) return processes;
    const q = procSearch.toLowerCase();
    return processes.filter((p) => p.name.toLowerCase().includes(q));
  }, [processes, procSearch]);

  const displayed = useMemo(() => {
    if (procTab === "cpu") {
      return [...filtered].sort((a, b) => b.cpu_usage - a.cpu_usage).slice(0, 15);
    }
    if (procTab === "ram") {
      return [...filtered].sort((a, b) => b.memory_bytes - a.memory_bytes).slice(0, 15);
    }
    return [...filtered].sort((a, b) => {
      const diff =
        sortCol === "cpu"
          ? a.cpu_usage - b.cpu_usage
          : sortCol === "mem"
            ? a.memory_bytes - b.memory_bytes
            : a.name.localeCompare(b.name);
      return sortDir === "desc" ? -diff : diff;
    });
  }, [filtered, procTab, sortCol, sortDir]);

  // Show spinner only if both qm and metrics haven't loaded yet
  if (!qm && loading) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6">
        <div className="flex flex-col gap-3 w-56">
          <p
            className="text-xs font-semibold uppercase tracking-widest text-center mb-2"
            style={{ color: "var(--text-3)" }}
          >
            {t.common_loading}
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

  if (!qm && error)
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3">
        <span className="text-sm" style={{ color: "var(--accent)" }}>
          {error ?? t.dash_load_error}
        </span>
        <button onClick={() => load()} className="btn-primary text-xs px-4 py-2">
          {t.common_retry}
        </button>
      </div>
    );

  const m = metrics;
  const charging = m ? m.battery_status === "charging" : false;

  // Find the selected interface by name; fallback to first available
  const activeNetRate =
    netRates.find((r) => r.name === selectedIface) ?? (netRates.length > 0 ? netRates[0] : null);
  const fallbackNet = m?.net_interfaces[0];

  const OVERSCAN = 8;
  const firstIdx = Math.max(0, Math.floor(vScrollTop / ROW_H) - OVERSCAN);
  const lastIdx = Math.min(
    displayed.length - 1,
    Math.ceil((vScrollTop + vListHeight) / ROW_H) + OVERSCAN
  );
  const topPad = firstIdx * ROW_H;
  const bottomPad = Math.max(0, (displayed.length - lastIdx - 1) * ROW_H);

  return (
    <div
      className="flex flex-col h-full px-5 pb-4 gap-3 overflow-hidden"
      style={{ position: "relative" }}
    >
      {/* ── Row 1 : Santé · CPU · GPU · Mémoire ── */}
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        className="grid grid-cols-4 gap-2.5"
        style={{ height: 178, flexShrink: 0 }}
      >
        {/* SANTÉ */}
        <div className="card p-4 overflow-hidden">
          <CardHeader icon={Sparkles} label={t.dash_health} color="var(--success)">
            {m && (
              <>
                {m.model && (
                  <span
                    className="text-[10px] whitespace-nowrap shrink-0"
                    style={{ color: "var(--text-3)" }}
                  >
                    {m.model}
                  </span>
                )}
                {m.mem_total > 0 && (
                  <span
                    className="text-[10px] whitespace-nowrap shrink-0"
                    style={{ color: "var(--text-3)" }}
                  >
                    {fmtBytesGo(m.mem_total)}
                  </span>
                )}
                {m.os_version && (
                  <span
                    className="text-[10px] whitespace-nowrap shrink-0"
                    style={{ color: "var(--text-3)" }}
                  >
                    {m.os_version}
                  </span>
                )}
              </>
            )}
          </CardHeader>
          <BigNum
            value={m && m.health_score > 0 ? `${m.health_score}` : "—"}
            sub={
              m
                ? m.health_score >= 80
                  ? t.dash_score_excellent
                  : m.health_score >= 60
                    ? t.dash_score_good
                    : t.dash_score_warn
                : undefined
            }
          />
          {m?.health_score_msg && (
            <p className="text-[11px] mt-1 truncate" style={{ color: "var(--text-3)" }}>
              {translateHealthMsg(m.health_score_msg, lang)}
            </p>
          )}
          <Detail>
            {m && (
              <span className="block mt-2">
                {t.dash_uptime} {m.uptime} · depuis {m.uptime}
              </span>
            )}
          </Detail>
        </div>

        {/* CPU */}
        <div className="card p-4 overflow-hidden">
          <CardHeader
            icon={Cpu}
            label="CPU"
            color="var(--cyan)"
            badge={m && m.thermal_cpu_temp > 0 ? `${m.thermal_cpu_temp.toFixed(0)}°C` : undefined}
          />
          <BigNum value={qm ? qm.cpu_usage.toFixed(1) : "—"} unit={qm ? "%" : undefined} />
          <CoreBlocks perCore={qm?.cpu_per_core ?? []} />
          <Detail>
            {qm && (
              <span className="block mt-1">
                Charge {qm.cpu_load1.toFixed(1)} / {qm.cpu_core_count} {"cœurs"} ·{" "}
                {qm.cpu_usage < 30 ? "inactif" : "actif"}
              </span>
            )}
          </Detail>
        </div>

        {/* GPU */}
        <div className="card p-4 overflow-hidden">
          <CardHeader
            icon={MonitorDot}
            label="GPU"
            color="var(--warning)"
            badge={
              gpuInfo
                ? gpuInfo.vram_mb > 0
                  ? `${(gpuInfo.vram_mb / 1024).toFixed(0)} Go VRAM`
                  : "Unifié"
                : undefined
            }
          />
          {gpuInfo ? (
            <>
              <BigNum
                value={qm ? `${qm.gpu_busy_percent.toFixed(0)}` : "—"}
                unit={qm ? "%" : undefined}
              />
              {gpuHistory.length > 1 && (
                <div className="mt-2">
                  <Sparkline data={gpuHistory} color="var(--warning)" height={28} />
                </div>
              )}
              <Detail>
                <span className="block mt-1">
                  {(qm?.gpu_busy_percent ?? 0) < 30 ? "normal" : "actif"} · {gpuInfo.model || "GPU"}
                </span>
              </Detail>
            </>
          ) : (
            <>
              <BigNum value="—" />
              <Detail>
                <span className="block mt-2">{t.dash_gpu_unavail}</span>
              </Detail>
            </>
          )}
        </div>

        {/* MÉMOIRE — clickable */}
        <div
          className="card p-4 overflow-hidden"
          onClick={() => setShowMemModal(true)}
          style={{ cursor: "pointer" }}
          onMouseEnter={(e) => (e.currentTarget.style.filter = "brightness(1.08)")}
          onMouseLeave={(e) => (e.currentTarget.style.filter = "")}
        >
          <CardHeader
            icon={Server}
            label={t.dash_memory}
            color="var(--warning)"
            badge={qm ? `Pression ${qm.mem_used_percent.toFixed(0)}%` : undefined}
          />
          <BigNum
            value={qm ? `${qm.mem_used_percent.toFixed(0)}` : "—"}
            unit={qm ? "%" : undefined}
          />
          {qm && (
            <FillBar
              pct={qm.mem_used_percent}
              used={qm.mem_used}
              total={qm.mem_total}
              color="var(--warning)"
            />
          )}
          <Detail>
            {qm && qm.mem_swap_total > 0 && (
              <span className="block mt-1">{fmtBytesGo(qm.mem_swap_used)} swap</span>
            )}
          </Detail>
        </div>
      </motion.div>

      {/* ── Row 2 : Batterie · Disque · Réseau · Ventilateur ── */}
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.05 }}
        className="grid grid-cols-4 gap-2.5"
        style={{ height: 200, flexShrink: 0 }}
      >
        {/* BATTERIE */}
        <div className="card p-4 overflow-hidden">
          <CardHeader
            icon={charging ? BatteryCharging : Battery}
            label={t.dash_battery}
            color="var(--success)"
            badge={m?.battery_health || undefined}
          />
          <BigNum
            value={m ? `${m.battery_percent}` : "—"}
            unit={m ? "%" : undefined}
            sub={
              m
                ? m.battery_time_left
                  ? m.battery_time_left + " " + t.dash_remaining
                  : m.battery_status
                : undefined
            }
          />
          {m && (
            <div className="mt-2">
              <MetricBar
                pct={m.battery_percent}
                color={m.battery_percent < 20 ? "var(--danger-text)" : "var(--success)"}
              />
            </div>
          )}
          <Detail>
            {m && (
              <>
                {m.top_processes[0] && (
                  <span className="block mt-1">{m.top_processes[0].name}</span>
                )}
                <span className="block">
                  {m.battery_cycles} {t.dash_cycles}
                  {m.thermal_battery_temp > 0 ? ` · ${m.thermal_battery_temp.toFixed(1)}°C` : ""}
                </span>
              </>
            )}
          </Detail>
          {m && m.bluetooth_devices.filter((d) => d.connected).length > 0 && (
            <div className="flex gap-1.5 mt-1 flex-wrap">
              {m.bluetooth_devices
                .filter((d) => d.connected)
                .map((d, i) => (
                  <div
                    key={i}
                    title={`${d.name}${d.battery ? ` · ${d.battery}` : ""}`}
                    className="flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px]"
                    style={{ background: "var(--accent-dim)", color: "var(--accent-text)" }}
                  >
                    <Bluetooth size={8} />
                    <span>{d.name}</span>
                  </div>
                ))}
            </div>
          )}
          <div className="flex items-center gap-2 mt-2">
            <button
              onClick={async () => {
                const newState = !lowPower;
                setLowPower(newState);
                lowPowerPending.current = newState;
                try {
                  await invoke("set_low_power_mode", { enable: newState });
                } catch {
                  setLowPower(!newState);
                } finally {
                  lowPowerPending.current = null;
                }
              }}
              className="flex items-center gap-1.5 text-[10px]"
              style={{ color: "var(--text-3)" }}
            >
              <span
                className="inline-block w-7 h-4 rounded-full transition-colors duration-200 relative"
                style={{ background: lowPower ? "var(--success)" : "var(--bar-track)" }}
              >
                <span
                  className="absolute top-0.5 left-0.5 w-3 h-3 rounded-full bg-white transition-transform duration-200"
                  style={{ transform: lowPower ? "translateX(12px)" : "translateX(0)" }}
                />
              </span>
              <span>{t.dash_power_save}</span>
              {lowPower && (
                <span
                  className="w-1.5 h-1.5 rounded-full inline-block"
                  style={{ background: "var(--success)" }}
                />
              )}
            </button>
          </div>
        </div>

        {/* DISQUE — navigate to Nettoyer */}
        <div
          className="card p-4 overflow-hidden"
          onClick={() => onNavigate?.("clean")}
          style={{ cursor: "pointer" }}
          onMouseEnter={(e) => (e.currentTarget.style.filter = "brightness(1.08)")}
          onMouseLeave={(e) => (e.currentTarget.style.filter = "")}
        >
          <CardHeader icon={HardDrive} label={t.dash_disk} color="var(--info)" />
          <div className="flex items-baseline gap-1 mt-0.5">
            <span
              className="text-2xl font-bold tracking-tight leading-none"
              style={{ color: "var(--text-1)" }}
            >
              {qm ? fmtBytesGo(qm.disk_total - qm.disk_used) : "—"}
            </span>
            {qm && (
              <span className="text-xs" style={{ color: "var(--text-3)" }}>
                {t.dash_free}
              </span>
            )}
          </div>
          {qm && (
            <FillBar
              pct={qm.disk_used_percent}
              used={qm.disk_used}
              total={qm.disk_total}
              color="var(--info)"
            />
          )}
          <Detail>
            {m && m.disk_io_read + m.disk_io_write > 0 && (
              <span className="block mt-1">
                ↓ {fmtMbs(m.disk_io_read)} · ↑ {fmtMbs(m.disk_io_write)}
              </span>
            )}
            {m && m.trash_size > 0 && (
              <span className="block">
                {t.dash_trash} {fmtBytesGo(m.trash_size)}
              </span>
            )}
          </Detail>
        </div>

        {/* RÉSEAU */}
        <div className="card p-4 overflow-hidden">
          <CardHeader icon={Wifi} label={t.dash_network} color="var(--cyan)">
            {netRates.length > 0 && (
              <div className="flex items-center gap-1">
                <button
                  onClick={() => {
                    const idx = netRates.findIndex((r) => r.name === selectedIface);
                    const prev = netRates[(idx - 1 + netRates.length) % netRates.length];
                    if (prev) switchIface(prev.name);
                  }}
                  style={{ color: "var(--text-3)", lineHeight: 1, fontSize: 10 }}
                >
                  ◀
                </button>
                <span className="text-[10px] font-mono" style={{ color: "var(--text-2)" }}>
                  {selectedIface}
                </span>
                <button
                  onClick={() => {
                    const idx = netRates.findIndex((r) => r.name === selectedIface);
                    const next = netRates[(idx + 1) % netRates.length];
                    if (next) switchIface(next.name);
                  }}
                  style={{ color: "var(--text-3)", lineHeight: 1, fontSize: 10 }}
                >
                  ▶
                </button>
              </div>
            )}
          </CardHeader>

          {activeNetRate ? (
            <>
              <div className="flex flex-col gap-0.5">
                <div className="flex items-baseline gap-1.5">
                  <span
                    className="text-lg font-bold leading-none"
                    style={{ color: "var(--text-1)" }}
                  >
                    {fmtBps(activeNetRate.rx_bps)}
                  </span>
                  <span className="text-[10px] font-semibold" style={{ color: "var(--cyan)" }}>
                    ↓
                  </span>
                </div>
                <div className="flex items-baseline gap-1.5">
                  <span
                    className="text-lg font-bold leading-none"
                    style={{ color: "var(--text-2)" }}
                  >
                    {fmtBps(activeNetRate.tx_bps)}
                  </span>
                  <span className="text-[10px] font-semibold" style={{ color: "var(--violet)" }}>
                    ↑
                  </span>
                </div>
              </div>
              <div className="mt-2">
                <BiDirSparkline rx={rxHistory} tx={txHistory} height={44} />
              </div>
              {activeNetRate &&
                (() => {
                  const ip = m?.net_interfaces.find((i) => i.name === selectedIface)?.ip;
                  return (
                    <Detail>
                      <span className="block mt-1">
                        {selectedIface}
                        {ip ? ` · ${ip}` : ""}
                      </span>
                    </Detail>
                  );
                })()}
            </>
          ) : fallbackNet ? (
            <>
              <div className="flex flex-col gap-0.5">
                <div className="flex items-baseline gap-1.5">
                  <span className="text-lg font-bold" style={{ color: "var(--text-1)" }}>
                    {fmtMbs(fallbackNet.rx_rate)}
                  </span>
                  <span className="text-[10px] font-semibold" style={{ color: "var(--cyan)" }}>
                    ↓
                  </span>
                </div>
                <div className="flex items-baseline gap-1.5">
                  <span className="text-lg font-bold" style={{ color: "var(--text-2)" }}>
                    {fmtMbs(fallbackNet.tx_rate)}
                  </span>
                  <span className="text-[10px] font-semibold" style={{ color: "var(--violet)" }}>
                    ↑
                  </span>
                </div>
              </div>
              <Detail>
                <span className="block mt-2">
                  {fallbackNet.name}
                  {fallbackNet.ip ? ` · ${fallbackNet.ip}` : ""}
                </span>
              </Detail>
            </>
          ) : (
            <BigNum value="—" />
          )}
        </div>

        {/* THERMIQUE */}
        <div className="card p-4 overflow-hidden">
          <CardHeader icon={Thermometer} label="Thermique" color="#f97316" />

          {/* 6 capteurs en grille 3×2 */}
          <div className="grid grid-cols-3 gap-1 mt-0.5">
            {(
              [
                {
                  label: "CPU",
                  val: qm?.cpu_temp,
                  color: "var(--cyan)",
                  title: "Cœurs CPU (pACC + eACC)",
                },
                { label: "GPU", val: qm?.gpu_temp, color: "var(--warning)", title: "GPU MTR" },
                {
                  label: "SoC",
                  val: qm?.soc_temp,
                  color: "var(--danger-text)",
                  title: "SOC Die / PMGR",
                },
                {
                  label: "SSD",
                  val: qm?.nand_temp,
                  color: "var(--info)",
                  title: "NAND — stockage",
                },
                {
                  label: "ANE",
                  val: qm?.ane_temp,
                  color: "var(--danger-text)",
                  title: "Apple Neural Engine",
                },
                {
                  label: "Bat",
                  val: m?.thermal_battery_temp,
                  color: "var(--success)",
                  title: "Batterie",
                },
              ] as { label: string; val: number | undefined; color: string; title: string }[]
            ).map(({ label, val, color, title }) => (
              <div
                key={label}
                className="flex flex-col items-center rounded-lg py-1.5"
                title={title}
                style={{ background: "var(--bg)" }}
              >
                <span
                  className="text-[9px] font-bold uppercase tracking-widest"
                  style={{ color: "var(--text-3)" }}
                >
                  {label}
                </span>
                <span
                  className="text-sm font-bold leading-tight mt-0.5"
                  style={{ color: typeof val === "number" && val > 0 ? color : "var(--text-3)" }}
                >
                  {typeof val === "number" && val > 0 ? `${val.toFixed(0)}°` : "—"}
                </span>
              </div>
            ))}
          </div>

          {/* Consommation — IOReport Energy Model, no root */}
          <div className="mt-2">
            <span
              className="text-[9px] font-bold uppercase tracking-widest"
              style={{ color: "var(--text-3)" }}
            >
              Consommation
            </span>
            <div className="grid grid-cols-2 gap-1 mt-1">
              {(
                [
                  { label: "CPU", val: qm?.cpu_power, title: "CPU" },
                  { label: "GPU", val: qm?.gpu_power, title: "GPU" },
                  { label: "RAM", val: qm?.ram_power, title: "DRAM — absent sur M1 Air" },
                  { label: "ANE", val: qm?.ane_power, title: "Neural Engine — 0W au repos" },
                ] as { label: string; val: number | undefined; title: string }[]
              ).map(({ label, val, title }) => (
                <div
                  key={label}
                  className="flex items-center justify-between rounded-lg px-2 py-1"
                  title={title}
                  style={{ background: "var(--bg)" }}
                >
                  <span
                    className="text-[9px] uppercase tracking-wide"
                    style={{ color: "var(--text-3)" }}
                  >
                    {label}
                  </span>
                  <span
                    className="text-[11px] font-bold"
                    style={{
                      color:
                        typeof val === "number" && val > 0.05 ? "var(--text-1)" : "var(--text-3)",
                    }}
                  >
                    {typeof val === "number" ? `${val.toFixed(1)}W` : "—"}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </motion.div>

      {/* ── Process table — prend l'espace restant, jamais moins que 160px ── */}
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.1 }}
        className="card overflow-hidden flex-1 flex flex-col"
        style={{ position: "relative", minHeight: 160 }}
      >
        {/* Kill confirmation dialog */}
        {killTarget !== null && (
          <motion.div
            initial={{ opacity: 0, scale: 0.92 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.92 }}
            style={{
              position: "absolute",
              inset: 0,
              zIndex: 50,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              background: "rgba(0,0,0,0.35)",
            }}
          >
            <div
              className="rounded-xl p-5 flex flex-col gap-3 shadow-xl"
              style={{
                background: "var(--bg-card)",
                border: "1px solid var(--border)",
                minWidth: 240,
              }}
            >
              <p className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
                {t.dash_kill_title}
              </p>
              <p className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {killTarget.name}
                {killTarget.pids.length > 1
                  ? ` (${killTarget.pids.length} instances)`
                  : ` (PID ${killTarget.pids[0]})`}
              </p>
              <div className="flex gap-2 justify-end mt-1">
                <button
                  onClick={() => setKillTarget(null)}
                  className="text-xs px-3 py-1.5 rounded-lg"
                  style={{
                    background: "var(--bg)",
                    color: "var(--text-2)",
                    border: "1px solid var(--border)",
                  }}
                >
                  {t.common_cancel}
                </button>
                <button
                  onClick={async () => {
                    await invoke("kill_process", { pid: killTarget.pids[0] }).catch(() => {});
                    setKillTarget(null);
                  }}
                  className="text-xs px-3 py-1.5 rounded-lg font-semibold"
                  style={{ background: "var(--danger)", color: "#fff" }}
                >
                  {t.dash_kill_confirm}
                </button>
              </div>
            </div>
          </motion.div>
        )}

        {/* Tabs + search header */}
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderBottom: "1px solid var(--border)" }}
        >
          <div
            className="flex items-center gap-0.5 p-0.5 rounded-lg"
            style={{ background: "var(--bg)" }}
          >
            {(["cpu", "ram", "all"] as const).map((tab) => (
              <button
                key={tab}
                onClick={() => setProcTab(tab)}
                className="text-[10px] font-semibold px-2.5 py-1 rounded-md transition-all"
                style={{
                  background: procTab === tab ? "var(--bg-card)" : "transparent",
                  color: procTab === tab ? "var(--text-1)" : "var(--text-3)",
                  border: procTab === tab ? "1px solid var(--border)" : "1px solid transparent",
                }}
              >
                {tab === "cpu" ? "Top CPU" : tab === "ram" ? "Top RAM" : "Tous"}
              </button>
            ))}
          </div>
          <input
            value={procSearch}
            onChange={(e) => setProcSearch(e.target.value)}
            placeholder="Rechercher..."
            className="flex-1 text-[11px] px-2.5 py-1 rounded-lg outline-none"
            style={{
              background: "var(--bg)",
              color: "var(--text-1)",
              border: "1px solid var(--border)",
              minWidth: 0,
            }}
          />
          <span className="text-[10px] shrink-0" style={{ color: "var(--text-3)" }}>
            {displayed.length}
            {procTab !== "all" ? "/15" : `/${filtered.length}`}
          </span>
        </div>

        {/* Column headers — only in "Tous" mode */}
        {procTab === "all" && (
          <table className="w-full text-[11px]" style={{ borderCollapse: "collapse" }}>
            <thead>
              <tr style={{ borderBottom: "1px solid var(--border)" }}>
                <th
                  className="text-left py-2 px-4 font-semibold cursor-pointer"
                  style={{ color: sortCol === "name" ? "var(--accent)" : "var(--text-3)" }}
                  onClick={() => toggleSort("name")}
                >
                  <div className="flex items-center gap-1.5">
                    <span>{t.dash_name_col}</span>
                    <SortIcon col="name" active={sortCol} dir={sortDir} />
                  </div>
                </th>
                <th
                  className="text-right py-2 px-4 font-semibold cursor-pointer w-40"
                  style={{ color: sortCol === "cpu" ? "var(--accent)" : "var(--text-3)" }}
                  onClick={() => toggleSort("cpu")}
                >
                  <div className="flex items-center justify-end gap-1.5">
                    <SortIcon col="cpu" active={sortCol} dir={sortDir} />
                    <span>CPU</span>
                  </div>
                </th>
                <th
                  className="text-right py-2 px-4 font-semibold cursor-pointer w-24"
                  style={{ color: sortCol === "mem" ? "var(--accent)" : "var(--text-3)" }}
                  onClick={() => toggleSort("mem")}
                >
                  <div className="flex items-center justify-end gap-1.5">
                    <SortIcon col="mem" active={sortCol} dir={sortDir} />
                    <span>RAM</span>
                  </div>
                </th>
                <th className="py-2 px-4 w-10" />
              </tr>
            </thead>
          </table>
        )}

        {/* Scrollable rows — virtual list */}
        <div
          ref={processListRef}
          className="overflow-y-auto flex-1"
          onScroll={(e) => setVScrollTop((e.currentTarget as HTMLDivElement).scrollTop)}
        >
          <table className="w-full text-[11px]" style={{ borderCollapse: "collapse" }}>
            <tbody>
              {topPad > 0 && (
                <tr>
                  <td colSpan={4} style={{ height: topPad, padding: 0, border: "none" }} />
                </tr>
              )}
              {displayed.slice(firstIdx, lastIdx + 1).map((p) => (
                <ProcessRow
                  key={p.name}
                  p={p}
                  icons={appIcons}
                  onKill={setKillTarget}
                  flashing={flashKeys.has(p.name)}
                />
              ))}
              {bottomPad > 0 && (
                <tr>
                  <td colSpan={4} style={{ height: bottomPad, padding: 0, border: "none" }} />
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </motion.div>

      {/* ── Free Memory modal ── */}
      {showMemModal && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: "rgba(0,0,0,0.4)",
            zIndex: 50,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            className="rounded-xl p-6 flex flex-col gap-4 shadow-2xl"
            style={{
              background: "var(--bg-card)",
              border: "1px solid var(--border)",
              width: 380,
              maxHeight: "70vh",
              position: "relative",
            }}
          >
            <button
              onClick={() => setShowMemModal(false)}
              style={{ position: "absolute", top: 12, right: 12, color: "var(--text-3)" }}
            >
              <X size={15} />
            </button>

            <p className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
              {t.dash_mem_title}
            </p>

            <div className="text-[11px] flex flex-col gap-1" style={{ color: "var(--text-2)" }}>
              <span>
                {t.dash_mem_used_lbl} {fmtBytes(qm?.mem_used ?? 0)} / {fmtBytes(qm?.mem_total ?? 0)}
              </span>
              <span>
                {t.dash_mem_avail_lbl} {m ? fmtBytes(m.mem_available) : "—"}
              </span>
              <MetricBar
                pct={qm?.mem_used_percent ?? 0}
                color={(qm?.mem_used_percent ?? 0) > 85 ? "var(--warning)" : undefined}
              />
            </div>

            <button
              onClick={free_memory_action}
              disabled={memFreeRunning}
              className="btn-primary text-xs px-4 py-2 self-start"
              style={{ opacity: memFreeRunning ? 0.6 : 1 }}
            >
              {memFreeRunning ? t.dash_mem_free_running : t.dash_mem_free_btn}
            </button>
          </motion.div>
        </div>
      )}

      {/* Toast notification bottom-right */}
      <AnimatePresence>
        {memFreeToast && (
          <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.95 }}
            transition={{ duration: 0.18 }}
            className="fixed bottom-4 right-4 z-50 flex items-center gap-2 px-4 py-2.5 rounded-xl text-[12px] font-medium shadow-lg"
            style={{
              background: memFreeToast.ok ? "var(--success)" : "var(--danger)",
              color: "#fff",
              boxShadow: `0 4px 16px ${memFreeToast.ok ? "var(--success-soft)" : "var(--danger-soft)"}`,
            }}
          >
            {memFreeToast.ok ? <Check size={13} /> : <AlertCircle size={13} />}
            {memFreeToast.msg}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
