import { useState } from "react";
import { motion } from "framer-motion";
import { PackageX, Clock, RotateCcw, Trash2 } from "lucide-react";
import { useT } from "../i18n/useT";
import { useMo } from "../hooks/useMo";

const projects = [
  { path: "~/Developer/mytube/frontend", artifacts: "node_modules", size: "890 MB", age: 2 },
  { path: "~/Developer/mytube/backend", artifacts: "target", size: "1.4 GB", age: 5 },
  { path: "~/Developer/old-app", artifacts: "node_modules, dist", size: "340 MB", age: 45 },
  { path: "~/Desktop/test-project", artifacts: "node_modules, build", size: "210 MB", age: 12 },
  { path: "~/Projects/rust-cli", artifacts: "target", size: "2.1 GB", age: 30 },
  { path: "~/Projects/burrow", artifacts: "node_modules", size: "180 MB", age: 0 },
];

export default function Purge() {
  const { t } = useT();
  const mo = useMo();
  const [selected, setSelected] = useState<Set<number>>(
    new Set(projects.map((_, i) => i).filter((i) => projects[i].age > 7))
  );

  const toggle = (i: number) =>
    setSelected((prev) => {
      const n = new Set(prev);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });

  const totalSize = projects
    .filter((_, i) => selected.has(i))
    .reduce((acc, p) => acc + parseFloat(p.size), 0)
    .toFixed(1);

  const handleRun = async () => {
    // Purge.tsx est une page inutilisée — l'opération purge n'est plus exposée
    console.warn("Purge operation not implemented");
  };

  return (
    <div className="flex flex-col h-full px-6 pt-6 pb-4 gap-4">
      <div className="flex items-center gap-3">
        <div
          className="w-10 h-10 rounded-2xl flex items-center justify-center"
          style={{ background: "rgba(255,0,0,0.15)", border: "1px solid rgba(255,0,0,0.25)" }}
        >
          <PackageX size={18} className="text-accent-light" />
        </div>
        <div>
          <h2 className="text-white font-bold text-lg">{t.purge_title}</h2>
          <p className="text-green-300/40 text-xs">{t.purge_sub}</p>
        </div>
        <div className="ml-auto glass rounded-lg px-3 py-1.5 text-xs text-green-300/50">
          {projects.length} {t.purge_scanned}
        </div>
      </div>

      <div className="flex flex-col gap-2 flex-1 overflow-y-auto min-h-0">
        {projects.map((p, i) => (
          <motion.button
            key={i}
            initial={{ opacity: 0, x: -8 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ delay: i * 0.05 }}
            onClick={() => toggle(i)}
            className={`glass rounded-xl px-4 py-3 flex items-center gap-3 text-left transition-all ${selected.has(i) ? "border-green-500/20" : "opacity-50"}`}
          >
            <div
              className={`w-4 h-4 rounded-full border-2 shrink-0 ${selected.has(i) ? "border-accent-light bg-accent-light/20" : "border-green-400/25"}`}
            />
            <div className="flex-1 min-w-0">
              <div className="text-white text-sm font-medium truncate">{p.path}</div>
              <div className="text-green-300/35 text-xs">{p.artifacts}</div>
            </div>
            {p.age <= 7 && (
              <span className="flex items-center gap-1 text-xs text-yellow-400/60 shrink-0">
                <Clock size={11} /> {p.age === 0 ? t.purge_today : `${p.age}${t.purge_days_ago}`}
              </span>
            )}
            <span className="text-accent-light text-sm font-semibold shrink-0">{p.size}</span>
          </motion.button>
        ))}
      </div>

      <div className="flex items-center gap-3">
        <div className="glass rounded-xl px-4 py-2.5 flex-1 text-center">
          <span className="text-green-300/50 text-xs">{t.purge_free} </span>
          <span className="text-white font-bold">{totalSize} GB</span>
        </div>
        <motion.button
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.97 }}
          onClick={handleRun}
          disabled={mo.status === "running" || selected.size === 0}
          className="flex items-center gap-2 px-6 py-2.5 rounded-xl text-white font-semibold text-sm shadow-glow-sm disabled:opacity-50"
          style={{ background: "var(--danger)" }}
        >
          {mo.status === "running" ? (
            <>
              <motion.div
                animate={{ rotate: 360 }}
                transition={{ repeat: Infinity, duration: 1, ease: "linear" }}
              >
                <RotateCcw size={14} />
              </motion.div>{" "}
              {t.purge_running}
            </>
          ) : mo.status === "done" ? (
            t.purge_done
          ) : (
            <>
              <Trash2 size={14} /> {t.purge_run}
            </>
          )}
        </motion.button>
      </div>
    </div>
  );
}
