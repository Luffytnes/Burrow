import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Archive, Trash2, RefreshCw, Loader2, CheckCircle2, AlertCircle, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n/useT";

interface InstallerFile {
  name: string;
  path: string;
  size_bytes: number;
  source: string;
}

// Hex bruts obligatoires : concaténés avec un alpha ("…18") plus bas
const sourceColor: Record<string, string> = {
  Downloads: "#c93a44",
  Desktop: "#4a90d9",
  Documents: "#8b6cc9",
  "Homebrew Cache": "#2f9e8f",
};

function fmtSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${(bytes / 1e3).toFixed(0)} KB`;
}

export default function Installer() {
  const { t } = useT();
  const [files, setFiles] = useState<InstallerFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [results, setResults] = useState<{ path: string; ok: boolean; err?: string }[]>([]);
  const [showResults, setShowResults] = useState(false);

  const loadFiles = useCallback(() => {
    setLoading(true);
    setSelected(new Set());
    setResults([]);
    setShowResults(false);
    invoke<InstallerFile[]>("list_installer_files")
      .then((list) => {
        setFiles(list);
        // Pre-select all by default
        setSelected(new Set(list.map((f) => f.path)));
      })
      .catch(() => setFiles([]))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    loadFiles();
  }, [loadFiles]);

  const toggle = (path: string) =>
    setSelected((prev) => {
      const n = new Set(prev);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });

  const toggleAll = () => {
    if (selected.size === files.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(files.map((f) => f.path)));
    }
  };

  const handleDelete = async () => {
    const toDelete = files.filter((f) => selected.has(f.path));
    if (toDelete.length === 0) return;

    setDeleting(true);
    setShowResults(true);
    const res: typeof results = [];

    for (const file of toDelete) {
      try {
        await invoke("move_to_trash", { path: file.path });
        res.push({ path: file.path, ok: true });
      } catch (e) {
        res.push({ path: file.path, ok: false, err: String(e) });
      }
      setResults([...res]);
    }

    setDeleting(false);
    // Remove successfully deleted files from list
    const deleted = new Set(res.filter((r) => r.ok).map((r) => r.path));
    setFiles((prev) => prev.filter((f) => !deleted.has(f.path)));
    setSelected((prev) => {
      const n = new Set(prev);
      deleted.forEach((p) => n.delete(p));
      return n;
    });
  };

  const totalSelected = files
    .filter((f) => selected.has(f.path))
    .reduce((acc, f) => acc + f.size_bytes, 0);

  return (
    <div className="flex flex-col h-full px-6 pt-6 pb-4 gap-4 relative">
      {/* Header */}
      <div className="flex items-center gap-3">
        <div
          className="w-10 h-10 rounded-2xl flex items-center justify-center"
          style={{ background: "rgba(255,0,0,0.15)", border: "1px solid rgba(255,0,0,0.25)" }}
        >
          <Archive size={18} className="text-accent-light" />
        </div>
        <div>
          <h2 className="text-white font-bold text-lg">{t.installer_title}</h2>
          <p className="text-green-300/40 text-xs">
            {loading
              ? t.installer_searching
              : files.length === 0
                ? t.installer_none
                : `${files.length} · ${fmtSize(files.reduce((a, f) => a + f.size_bytes, 0))} ${t.common_total}`}
          </p>
        </div>
        <button
          onClick={loadFiles}
          disabled={loading || deleting}
          className="ml-auto text-green-400/40 hover:text-green-300 transition-colors disabled:opacity-30"
        >
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {/* File list */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {loading ? (
          <div className="flex items-center justify-center h-full gap-2 text-green-300/40">
            <Loader2 size={15} className="animate-spin" />
            <span className="text-sm">{t.installer_scanning}</span>
          </div>
        ) : files.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-2 text-green-300/30">
            <CheckCircle2 size={24} className="text-green-400/30" />
            <span className="text-sm">{t.installer_empty}</span>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {/* Select all toggle */}
            <button
              onClick={toggleAll}
              className="glass rounded-xl px-4 py-2.5 flex items-center gap-3 text-left hover:bg-white/3 transition-colors"
            >
              <div
                className={`w-4 h-4 rounded-full border-2 shrink-0 transition-colors ${
                  selected.size === files.length
                    ? "border-accent-light bg-accent-light/20"
                    : selected.size > 0
                      ? "border-accent-light/50 bg-accent-light/10"
                      : "border-green-400/25"
                }`}
              />
              <span className="text-green-300/60 text-sm">
                {selected.size === files.length ? t.common_deselect_all : t.common_select_all}
              </span>
              <span className="ml-auto text-green-300/30 text-xs">
                {selected.size} / {files.length}
              </span>
            </button>

            {files.map((f, i) => (
              <motion.button
                key={f.path}
                initial={{ opacity: 0, x: -6 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: Math.min(i * 0.04, 0.25) }}
                onClick={() => toggle(f.path)}
                className={`glass rounded-xl px-4 py-3 flex items-center gap-3 text-left transition-all ${
                  selected.has(f.path) ? "border-red-500/20" : "opacity-50"
                }`}
              >
                <div
                  className={`w-4 h-4 rounded-full border-2 shrink-0 transition-colors ${
                    selected.has(f.path) ? "border-red-400/70 bg-red-400/15" : "border-green-400/25"
                  }`}
                />
                <div className="flex-1 min-w-0">
                  <div className="text-white text-sm font-medium truncate">{f.name}</div>
                  <span
                    className="inline-block text-xs px-1.5 py-0.5 rounded-md mt-0.5"
                    style={{
                      background: `${sourceColor[f.source] ?? "#c93a44"}18`,
                      color: sourceColor[f.source] ?? "#c93a44",
                    }}
                  >
                    {f.source}
                  </span>
                </div>
                <span className="text-red-400/70 text-sm font-semibold shrink-0">
                  {fmtSize(f.size_bytes)}
                </span>
              </motion.button>
            ))}
          </div>
        )}
      </div>

      {/* Footer: total + delete button */}
      {files.length > 0 && (
        <div className="flex items-center gap-3">
          <div className="glass rounded-xl px-4 py-2.5 flex-1 text-center">
            <span className="text-green-300/50 text-xs">{t.installer_selected} </span>
            <span className="text-white font-bold">{fmtSize(totalSelected)}</span>
          </div>
          <motion.button
            whileHover={!deleting && selected.size > 0 ? { scale: 1.02 } : {}}
            whileTap={!deleting && selected.size > 0 ? { scale: 0.97 } : {}}
            onClick={handleDelete}
            disabled={deleting || selected.size === 0}
            className="flex items-center gap-2 px-6 py-2.5 rounded-xl text-white font-semibold text-sm disabled:opacity-50 transition-opacity"
            style={{ background: "var(--danger)" }}
          >
            {deleting ? (
              <>
                <Loader2 size={14} className="animate-spin" /> {t.installer_deleting}
              </>
            ) : (
              <>
                <Trash2 size={14} /> {t.installer_move_trash}
              </>
            )}
          </motion.button>
        </div>
      )}

      {/* Results panel */}
      <AnimatePresence>
        {showResults && results.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: 16, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.97 }}
            transition={{ duration: 0.2 }}
            className="absolute inset-x-6 bottom-24 glass rounded-2xl overflow-hidden"
            style={{ border: "1px solid rgba(239,68,68,0.2)" }}
          >
            <div className="flex items-center justify-between px-4 py-2 border-b border-white/5">
              <span className="text-green-300/60 text-xs font-semibold">
                {deleting
                  ? t.installer_progress
                  : `${results.filter((r) => r.ok).length} / ${results.length} ${t.installer_moved}`}
              </span>
              {!deleting && (
                <button
                  onClick={() => setShowResults(false)}
                  className="text-green-400/30 hover:text-green-300/60 transition-colors"
                >
                  <X size={12} />
                </button>
              )}
            </div>

            <div className="p-3 max-h-28 overflow-y-auto space-y-0.5">
              {results.map((r, i) => {
                const name = r.path.split("/").pop() ?? r.path;
                return (
                  <div key={i} className="flex items-center gap-2 text-xs font-mono">
                    {r.ok ? (
                      <CheckCircle2 size={10} className="text-green-400/70 shrink-0" />
                    ) : (
                      <AlertCircle size={10} className="text-red-400/70 shrink-0" />
                    )}
                    <span className={`truncate ${r.ok ? "text-green-300/60" : "text-red-400/60"}`}>
                      {name}
                    </span>
                    {r.err && (
                      <span className="text-red-400/40 shrink-0 truncate max-w-24">{r.err}</span>
                    )}
                  </div>
                );
              })}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
