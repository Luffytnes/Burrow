import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Zap,
  Wifi,
  Search,
  Monitor,
  LayoutGrid,
  Cpu,
  Loader2,
  CheckCircle2,
  AlertCircle,
  HardDrive,
  Database,
  ShieldCheck,
} from "lucide-react";
import { useMo } from "../hooks/useMo";
import { useT } from "../i18n/useT";

function getTasks(t: Record<string, string>) {
  return [
    { id: "finder", label: t.task_finder, desc: t.task_finder_desc, icon: Monitor, admin: false },
    { id: "dock", label: t.task_dock, desc: t.task_dock_desc, icon: LayoutGrid, admin: false },
    { id: "dns", label: t.task_dns, desc: t.task_dns_desc, icon: Wifi, admin: true },
    {
      id: "spotlight",
      label: t.task_spotlight,
      desc: t.task_spotlight_desc,
      icon: Search,
      admin: true,
    },
    { id: "swap", label: t.task_swap, desc: t.task_swap_desc, icon: Cpu, admin: true },
    {
      id: "launchpad",
      label: t.task_launchpad,
      desc: t.task_launchpad_desc,
      icon: LayoutGrid,
      admin: false,
    },
    {
      id: "periodic",
      label: t.task_periodic,
      desc: t.task_periodic_desc,
      icon: ShieldCheck,
      admin: true,
    },
    {
      id: "diskutil_verify",
      label: t.task_diskutil_verify,
      desc: t.task_diskutil_verify_desc,
      icon: HardDrive,
      admin: false,
    },
    {
      id: "launch_services",
      label: t.task_launch_services,
      desc: t.task_launch_services_desc,
      icon: Database,
      admin: false,
    },
  ];
}

interface TaskRowProps {
  checked: boolean;
  onChange: () => void;
  icon: React.ElementType;
  label: string;
  desc: string;
  admin: boolean;
}

function TaskRow({ checked, onChange, icon: Icon, label, desc, admin }: TaskRowProps) {
  return (
    <button
      onClick={onChange}
      className="w-full flex items-center gap-3 px-4 py-3 text-left transition-colors"
      style={{
        borderLeft: checked ? "2px solid var(--accent)" : "2px solid transparent",
        background: "transparent",
      }}
    >
      {checked ? (
        <Zap size={16} style={{ color: "var(--accent)", flexShrink: 0 }} />
      ) : (
        <div
          className="w-4 h-4 rounded-full shrink-0"
          style={{ border: "1.5px solid var(--text-3)" }}
        />
      )}
      <Icon size={15} style={{ color: "var(--text-3)", flexShrink: 0 }} />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
            {label}
          </span>
          {admin && (
            <span
              className="text-[10px] font-semibold px-1.5 py-0.5 rounded"
              style={{ background: "var(--warning)", color: "#1c1917" }}
            >
              admin
            </span>
          )}
        </div>
        <div className="text-[11px] truncate" style={{ color: "var(--text-3)" }}>
          {desc}
        </div>
      </div>
    </button>
  );
}

export default function Optimize() {
  const { t } = useT();
  const TASKS = getTasks(t);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [toast, setToast] = useState<{ ok: boolean; msg: string } | null>(null);
  const mo = useMo();

  const isRunning = mo.status === "running";

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleRun = async () => {
    mo.reset();
    const count = selected.size;
    const code = await mo.runCmd("run_optimize_selection", { tasks: Array.from(selected) });
    const msg =
      code === 0
        ? `${count} optimisation${count > 1 ? "s" : ""} appliquée${count > 1 ? "s" : ""}`
        : "Erreur lors de l'optimisation";
    setToast({ ok: code === 0, msg });
    setTimeout(() => setToast(null), 3500);
  };

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-3 relative">
      <div className="flex-1 overflow-y-auto flex flex-col gap-2 min-h-0">
        <div className="flex items-center justify-between px-1">
          <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
            {selected.size > 0 ? `${selected.size} / ${TASKS.length}` : t.common_none_selected}
          </span>
          <button
            onClick={() => {
              if (selected.size === TASKS.length) {
                setSelected(new Set());
              } else {
                setSelected(new Set(TASKS.map((task) => task.id)));
              }
            }}
            className="text-[11px] font-medium transition-colors"
            style={{ color: "var(--accent)" }}
          >
            {selected.size === TASKS.length ? t.common_deselect_all : t.common_select_all}
          </button>
        </div>
        <div className="card overflow-hidden">
          {TASKS.map((task, i) => (
            <div key={task.id} style={i > 0 ? { borderTop: "1px solid var(--border)" } : {}}>
              <TaskRow
                checked={selected.has(task.id)}
                onChange={() => toggle(task.id)}
                icon={task.icon}
                label={task.label}
                desc={task.desc}
                admin={task.admin}
              />
            </div>
          ))}
        </div>
      </div>

      <button
        onClick={handleRun}
        disabled={isRunning || selected.size === 0}
        className="btn-primary flex items-center justify-center gap-2 shrink-0 disabled:opacity-50"
      >
        {isRunning ? (
          <>
            <Loader2 size={14} className="animate-spin" /> {t.optimize_running}
          </>
        ) : (
          <>
            <Zap size={14} /> {t.optimize_run} ({selected.size})
          </>
        )}
      </button>

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
