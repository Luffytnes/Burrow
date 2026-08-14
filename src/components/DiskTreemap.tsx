import { useCallback, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  ChevronLeft,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  LayoutGrid,
  List,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";

interface DiskEntry {
  name: string;
  path: string;
  size_bytes: number;
  is_dir: boolean;
}

interface DiskBreakdownResult {
  entries: DiskEntry[];
  truncated: boolean;
}

type DiskBrowseError = "protected" | "inaccessible" | "busy" | "timeout" | "changed" | "system";

interface TreemapRect {
  entry: DiskEntry;
  x: number;
  y: number;
  width: number;
  height: number;
}

function fmtBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} Go`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} Mo`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} Ko`;
  return `${bytes} o`;
}

function fileExtColor(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mov", "mkv", "avi", "m4v"].includes(ext)) return "var(--danger)";
  if (["jpg", "jpeg", "png", "gif", "heic", "raw", "cr2"].includes(ext)) return "var(--success)";
  if (["zip", "tar", "gz", "rar", "7z", "dmg", "pkg"].includes(ext)) return "var(--warning)";
  if (["pdf", "doc", "docx", "pages", "xls", "xlsx"].includes(ext)) return "var(--info)";
  if (["mp3", "flac", "aac", "wav", "m4a"].includes(ext)) return "var(--pink)";
  return "var(--text-3)";
}

function binaryTreemap(
  entries: DiskEntry[],
  x = 0,
  y = 0,
  width = 100,
  height = 100
): TreemapRect[] {
  if (entries.length === 0) return [];
  if (entries.length === 1) return [{ entry: entries[0], x, y, width, height }];
  const total = entries.reduce((sum, entry) => sum + entry.size_bytes, 0);
  let leftTotal = 0;
  let split = 1;
  for (let index = 0; index < entries.length - 1; index++) {
    leftTotal += entries[index].size_bytes;
    split = index + 1;
    if (leftTotal >= total / 2) break;
  }
  const ratio = total > 0 ? leftTotal / total : 0.5;
  if (width >= height) {
    const leftWidth = width * ratio;
    return [
      ...binaryTreemap(entries.slice(0, split), x, y, leftWidth, height),
      ...binaryTreemap(entries.slice(split), x + leftWidth, y, width - leftWidth, height),
    ];
  }
  const topHeight = height * ratio;
  return [
    ...binaryTreemap(entries.slice(0, split), x, y, width, topHeight),
    ...binaryTreemap(entries.slice(split), x, y + topHeight, width, height - topHeight),
  ];
}

function errorLabel(error: DiskBrowseError): string {
  switch (error) {
    case "protected":
      return "Ce dossier est protégé et ne peut pas être analysé.";
    case "inaccessible":
      return "Ce dossier n’est pas accessible.";
    case "busy":
      return "Une analyse de stockage est déjà en cours.";
    case "timeout":
      return "Ce dossier est trop volumineux. Choisissez un sous-dossier plus précis.";
    case "changed":
      return "Le dossier a changé pendant l’analyse. Vous pouvez réessayer.";
    default:
      return "L’analyse du dossier n’a pas pu être terminée.";
  }
}

export default function DiskTreemap() {
  const [stack, setStack] = useState<string[]>([]);
  const [entries, setEntries] = useState<DiskEntry[] | null>(null);
  const [browseError, setBrowseError] = useState<DiskBrowseError | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [pickingFolder, setPickingFolder] = useState(false);
  const [view, setView] = useState<"treemap" | "list">("treemap");

  const load = useCallback(async (path: string) => {
    if (!path) return;
    setLoading(true);
    setBrowseError(null);
    setTruncated(false);
    try {
      const result = await invoke<DiskBreakdownResult>("get_disk_breakdown", { path });
      setEntries(result.entries);
      setTruncated(result.truncated);
    } catch (error) {
      setEntries([]);
      const message = typeof error === "string" ? error : "system:unknown";
      const kind = message.split(":")[0] as DiskBrowseError;
      setBrowseError(
        ["protected", "inaccessible", "busy", "timeout", "changed"].includes(kind) ? kind : "system"
      );
    } finally {
      setLoading(false);
    }
  }, []);

  const pickFolder = async () => {
    setPickingFolder(true);
    try {
      const path = await invoke<string | null>("pick_folder");
      if (!path) return;
      setStack([path]);
      await load(path);
    } finally {
      setPickingFolder(false);
    }
  };

  const currentPath = stack[stack.length - 1] ?? "";
  const totalSize = entries?.reduce((sum, entry) => sum + entry.size_bytes, 0) ?? 0;
  const rectangles = useMemo(() => binaryTreemap((entries ?? []).slice(0, 48)), [entries]);

  const navigate = (entry: DiskEntry) => {
    if (!entry.is_dir || loading) return;
    const next = [...stack, entry.path];
    setStack(next);
    void load(entry.path);
  };

  const goBack = () => {
    if (stack.length <= 1 || loading) return;
    const next = stack.slice(0, -1);
    setStack(next);
    void load(next[next.length - 1]);
  };

  if (stack.length === 0) {
    return (
      <div className="card flex flex-col items-center justify-center gap-3 py-12 px-6 text-center">
        <div
          className="w-12 h-12 rounded-2xl flex items-center justify-center"
          style={{ background: "var(--accent-dim)", color: "var(--accent)" }}
        >
          <LayoutGrid size={21} />
        </div>
        <div>
          <div className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
            Visualiser l’occupation d’un dossier
          </div>
          <p className="text-[11px] mt-1 max-w-md" style={{ color: "var(--text-3)" }}>
            L’analyse ne démarre jamais automatiquement. Choisissez un dossier précis pour obtenir
            une treemap rapide, puis ouvrez ses sous-dossiers pour affiner.
          </p>
        </div>
        <button
          onClick={pickFolder}
          disabled={pickingFolder}
          className="btn-primary flex items-center gap-2 px-4 py-2 disabled:opacity-50"
        >
          {pickingFolder ? (
            <Loader2 size={13} className="animate-spin" />
          ) : (
            <FolderOpen size={13} />
          )}
          Choisir un dossier
        </button>
      </div>
    );
  }

  return (
    <div className="card overflow-hidden">
      <div
        className="flex items-center gap-2 px-3 py-2"
        style={{ borderBottom: "1px solid var(--border)", background: "var(--bg)" }}
      >
        <button
          onClick={goBack}
          disabled={stack.length <= 1 || loading}
          className="p-1 rounded disabled:opacity-30"
          style={{ color: "var(--text-3)" }}
          aria-label="Dossier précédent"
        >
          <ChevronLeft size={13} />
        </button>
        <Folder size={12} style={{ color: "var(--info)", flexShrink: 0 }} />
        <span className="text-[10px] truncate flex-1" style={{ color: "var(--text-2)" }}>
          {currentPath.replace(/^\/Users\/[^/]+/, "~")}
        </span>
        {loading && (
          <Loader2 size={11} className="animate-spin" style={{ color: "var(--accent)" }} />
        )}
        <button
          onClick={() => void load(currentPath)}
          disabled={loading}
          className="p-1 rounded disabled:opacity-30"
          style={{ color: "var(--text-3)" }}
          aria-label="Actualiser l’analyse"
        >
          <RefreshCw size={11} />
        </button>
        <button
          onClick={pickFolder}
          disabled={pickingFolder || loading}
          className="text-[10px] px-2 py-1 rounded-lg flex items-center gap-1 disabled:opacity-40"
          style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
        >
          <FolderOpen size={10} /> Changer
        </button>
        <div
          className="flex items-center p-0.5 rounded-lg"
          style={{ background: "var(--bar-track)" }}
        >
          <button
            onClick={() => setView("treemap")}
            className="p-1 rounded-md"
            style={{
              color: view === "treemap" ? "var(--accent)" : "var(--text-3)",
              background: view === "treemap" ? "var(--bg-card)" : "transparent",
            }}
            aria-label="Vue treemap"
          >
            <LayoutGrid size={11} />
          </button>
          <button
            onClick={() => setView("list")}
            className="p-1 rounded-md"
            style={{
              color: view === "list" ? "var(--accent)" : "var(--text-3)",
              background: view === "list" ? "var(--bg-card)" : "transparent",
            }}
            aria-label="Vue liste"
          >
            <List size={11} />
          </button>
        </div>
      </div>

      {loading && entries === null ? (
        <div
          className="flex items-center justify-center gap-2 py-12"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={15} className="animate-spin" />
          <span className="text-xs">Analyse du dossier…</span>
        </div>
      ) : browseError ? (
        <div className="flex flex-col items-center gap-3 py-10 px-4 text-center">
          <AlertMessage>{errorLabel(browseError)}</AlertMessage>
          <div className="flex gap-2">
            <button
              onClick={() => void load(currentPath)}
              className="text-[11px] px-3 py-1.5 rounded-lg"
              style={{ background: "var(--bar-track)", color: "var(--text-2)" }}
            >
              Réessayer
            </button>
            <button
              onClick={pickFolder}
              className="text-[11px] px-3 py-1.5 rounded-lg"
              style={{ background: "var(--accent)", color: "var(--on-accent)" }}
            >
              Choisir un autre dossier
            </button>
          </div>
        </div>
      ) : (entries?.length ?? 0) === 0 ? (
        <div className="text-center py-10 text-xs" style={{ color: "var(--text-3)" }}>
          Ce dossier est vide.
        </div>
      ) : view === "treemap" ? (
        <div
          className="relative m-2 rounded-lg overflow-hidden"
          style={{ height: 340, background: "var(--bar-track)" }}
        >
          {rectangles.map(({ entry, x, y, width, height }, index) => {
            const color = entry.is_dir ? "var(--info)" : fileExtColor(entry.name);
            const showLabel = width >= 10 && height >= 12;
            return (
              <motion.button
                key={entry.path}
                initial={{ opacity: 0, scale: 0.97 }}
                animate={{ opacity: 1, scale: 1 }}
                transition={{ delay: Math.min(index * 0.01, 0.3) }}
                onClick={() => navigate(entry)}
                disabled={!entry.is_dir}
                title={`${entry.name} — ${fmtBytes(entry.size_bytes)}`}
                className="absolute overflow-hidden text-left p-2 disabled:cursor-default"
                style={{
                  left: `${x}%`,
                  top: `${y}%`,
                  width: `${width}%`,
                  height: `${height}%`,
                  background: `color-mix(in srgb, ${color} 24%, var(--bg-card))`,
                  border: "1px solid var(--bg)",
                  color: "var(--text-1)",
                }}
              >
                {showLabel && (
                  <>
                    <span className="text-[10px] font-semibold block truncate">{entry.name}</span>
                    <span className="text-[9px] block mt-0.5" style={{ color: "var(--text-3)" }}>
                      {fmtBytes(entry.size_bytes)}
                    </span>
                  </>
                )}
              </motion.button>
            );
          })}
        </div>
      ) : (
        <div style={{ maxHeight: 350, overflowY: "auto" }}>
          {truncated && (
            <div className="px-3 py-2 text-[10px]" style={{ color: "var(--warning)" }}>
              Affichage limité. Ouvrez un sous-dossier pour affiner l’analyse.
            </div>
          )}
          {entries!.map((entry, index) => {
            const ratio = totalSize > 0 ? (entry.size_bytes / totalSize) * 100 : 0;
            const color = entry.is_dir ? "var(--info)" : fileExtColor(entry.name);
            return (
              <button
                key={entry.path}
                onClick={() => navigate(entry)}
                disabled={!entry.is_dir}
                className="w-full flex items-center gap-2.5 px-3 py-2 text-left"
                style={{ borderTop: index > 0 ? "1px solid var(--border)" : "none" }}
              >
                {entry.is_dir ? (
                  <Folder size={12} style={{ color }} />
                ) : (
                  <File size={12} style={{ color }} />
                )}
                <span className="text-[11px] flex-1 truncate" style={{ color: "var(--text-1)" }}>
                  {entry.name}
                </span>
                <div className="w-24 h-1 rounded-full" style={{ background: "var(--bar-track)" }}>
                  <div
                    className="h-full rounded-full"
                    style={{ width: `${Math.min(ratio, 100)}%`, background: color }}
                  />
                </div>
                <span
                  className="text-[10px] font-mono w-16 text-right"
                  style={{ color: "var(--text-3)" }}
                >
                  {fmtBytes(entry.size_bytes)}
                </span>
                {entry.is_dir && <ChevronRight size={10} style={{ color: "var(--text-3)" }} />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function AlertMessage({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-xs max-w-md" style={{ color: "var(--text-3)" }}>
      {children}
    </p>
  );
}
