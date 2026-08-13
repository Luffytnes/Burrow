import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity as ActivityIcon,
  CheckCircle2,
  Clock3,
  RotateCcw,
  Trash2,
  XCircle,
} from "lucide-react";

interface ActivityEntry {
  id: string;
  timestamp: number;
  category: string;
  action: string;
  status: string;
  summary: string;
  bytes: number | null;
  reversible: boolean;
}

function fmtBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} Go`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} Mo`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} Ko`;
  return `${bytes} o`;
}

export default function Activity() {
  const [entries, setEntries] = useState<ActivityEntry[]>([]);
  const [filter, setFilter] = useState("all");
  const [loading, setLoading] = useState(true);
  const [confirmClear, setConfirmClear] = useState(false);

  const load = useCallback(async () => {
    try {
      setEntries(await invoke<ActivityEntry[]>("list_activity", { limit: 500 }));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  }, [load]);

  const categories = useMemo(
    () => Array.from(new Set(entries.map((entry) => entry.category))).sort(),
    [entries]
  );
  const visible = filter === "all" ? entries : entries.filter((entry) => entry.category === filter);

  const clear = async () => {
    await invoke("clear_activity");
    setEntries([]);
    setConfirmClear(false);
  };

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-4">
      <div className="flex items-center gap-3 pt-1">
        <div
          className="w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <ActivityIcon size={16} style={{ color: "var(--accent)" }} />
        </div>
        <div className="flex-1">
          <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
            Journal d’activité
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            Historique local des analyses, nettoyages et opérations sensibles
          </p>
        </div>
        <button
          onClick={load}
          className="p-2 rounded-lg"
          style={{ color: "var(--text-3)", background: "var(--bg-card)" }}
          aria-label="Actualiser le journal"
        >
          <RotateCcw size={13} className={loading ? "animate-spin" : ""} />
        </button>
        {confirmClear ? (
          <div className="flex items-center gap-1.5">
            <button
              onClick={clear}
              className="text-[11px] px-3 py-1.5 rounded-lg font-semibold"
              style={{ background: "var(--danger)", color: "#fff" }}
            >
              Confirmer
            </button>
            <button
              onClick={() => setConfirmClear(false)}
              className="text-[11px] px-3 py-1.5 rounded-lg"
              style={{ background: "var(--bg-card)", color: "var(--text-2)" }}
            >
              Annuler
            </button>
          </div>
        ) : (
          <button
            onClick={() => setConfirmClear(true)}
            disabled={entries.length === 0}
            className="p-2 rounded-lg disabled:opacity-30"
            style={{ color: "var(--danger)", background: "var(--bg-card)" }}
            aria-label="Effacer le journal"
          >
            <Trash2 size={13} />
          </button>
        )}
      </div>

      <div className="flex items-center gap-1 overflow-x-auto shrink-0">
        {["all", ...categories].map((category) => (
          <button
            key={category}
            onClick={() => setFilter(category)}
            className="text-[10px] px-3 py-1.5 rounded-full whitespace-nowrap capitalize"
            style={{
              background: filter === category ? "var(--accent)" : "var(--bg-card)",
              color: filter === category ? "var(--on-accent)" : "var(--text-3)",
              border: "1px solid var(--border)",
            }}
          >
            {category === "all" ? "Tout" : category}
          </button>
        ))}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto card">
        {loading && entries.length === 0 ? (
          <div
            className="h-full flex items-center justify-center text-xs"
            style={{ color: "var(--text-3)" }}
          >
            Chargement du journal…
          </div>
        ) : visible.length === 0 ? (
          <div
            className="h-full flex flex-col items-center justify-center gap-2"
            style={{ color: "var(--text-3)" }}
          >
            <Clock3 size={24} />
            <span className="text-sm">Aucune activité enregistrée</span>
          </div>
        ) : (
          visible.map((entry, index) => {
            const success = entry.status === "success";
            return (
              <div
                key={entry.id}
                className="flex items-start gap-3 px-4 py-3"
                style={{ borderTop: index > 0 ? "1px solid var(--border)" : undefined }}
              >
                {success ? (
                  <CheckCircle2 size={15} style={{ color: "var(--success)", marginTop: 2 }} />
                ) : (
                  <XCircle size={15} style={{ color: "var(--danger)", marginTop: 2 }} />
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
                      {entry.action}
                    </span>
                    <span
                      className="text-[9px] uppercase tracking-wide px-1.5 py-0.5 rounded-full"
                      style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                    >
                      {entry.category}
                    </span>
                    {entry.reversible && (
                      <span className="text-[9px]" style={{ color: "var(--success)" }}>
                        ↩ récupérable
                      </span>
                    )}
                  </div>
                  <div className="text-[11px] truncate mt-0.5" style={{ color: "var(--text-2)" }}>
                    {entry.summary}
                  </div>
                  <div className="text-[10px] mt-1" style={{ color: "var(--text-3)" }}>
                    {new Date(entry.timestamp * 1000).toLocaleString()}
                    {entry.bytes != null && ` · ${fmtBytes(entry.bytes)}`}
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
