import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  CheckCircle2,
  CheckSquare2,
  Circle,
  Gauge,
  Loader2,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  Wrench,
} from "lucide-react";

interface CleanCategorySize {
  id: string;
  size_mb: number;
}

interface SmartScanResult {
  clean_categories: CleanCategorySize[];
  safe_clean_bytes: number;
  clamav_ready: boolean;
  definitions_outdated: boolean;
  maintenance_recommendations: string[];
  disk_used_percent: number;
}

type Stage = "idle" | "running" | "ready" | "fixing" | "done";

const CLEAN_LABELS: Record<string, string> = {
  user_cache: "Caches utilisateur",
  system_logs: "Journaux système",
  crash_reports: "Rapports de crash",
  npm_cache: "Cache npm",
  yarn_cache: "Cache Yarn",
  browser_cache: "Caches navigateurs",
  xcode: "Xcode DerivedData",
  brew_cache: "Cache Homebrew",
  simulator: "Caches simulateurs",
};

const TASK_LABELS: Record<string, string> = {
  dns: "Actualiser le cache DNS",
  diskutil_verify: "Vérifier le volume système",
  swap: "Soulager la mémoire",
  tmutil_thin: "Alléger les snapshots Time Machine",
};

function fmtBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} Go`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} Mo`;
  return `${Math.round(bytes / 1e3)} Ko`;
}

async function prepareEvent(name: string) {
  let resolveEvent: (value: number) => void = () => {};
  const promise = new Promise<number>((resolve) => {
    resolveEvent = resolve;
  });
  const unlisten = await listen<number>(name, (event) => {
    unlisten();
    resolveEvent(event.payload);
  });
  return { promise, unlisten };
}

export default function SmartScan() {
  const [stage, setStage] = useState<Stage>("idle");
  const [result, setResult] = useState<SmartScanResult | null>(null);
  const [securityDone, setSecurityDone] = useState(false);
  const [threats, setThreats] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [selectedCleanCategories, setSelectedCleanCategories] = useState<Set<string>>(new Set());
  const [selectedMaintenanceTasks, setSelectedMaintenanceTasks] = useState<Set<string>>(new Set());
  const [completedCleanCategories, setCompletedCleanCategories] = useState<Set<string>>(new Set());
  const [completedMaintenanceTasks, setCompletedMaintenanceTasks] = useState<Set<string>>(
    new Set()
  );

  const safeCategories = useMemo(
    () =>
      (result?.clean_categories ?? []).filter(
        (category) =>
          category.size_mb > 0 && category.id !== "trash" && category.id !== "ios_backups"
      ),
    [result]
  );

  const remainingCleanCategories = safeCategories.filter(
    (category) => !completedCleanCategories.has(category.id)
  );
  const remainingMaintenanceTasks = (result?.maintenance_recommendations ?? []).filter(
    (task) => !completedMaintenanceTasks.has(task)
  );
  const selectedCleanCount = remainingCleanCategories.filter((category) =>
    selectedCleanCategories.has(category.id)
  ).length;
  const selectedMaintenanceCount = remainingMaintenanceTasks.filter((task) =>
    selectedMaintenanceTasks.has(task)
  ).length;
  const selectedActionCount = selectedCleanCount + selectedMaintenanceCount;
  const remainingActionCount = remainingCleanCategories.length + remainingMaintenanceTasks.length;

  const toggleSelection = (
    id: string,
    setter: React.Dispatch<React.SetStateAction<Set<string>>>
  ) => {
    setter((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const scan = async () => {
    setStage("running");
    setResult(null);
    setSecurityDone(false);
    setThreats(0);
    setError(null);
    setNotice(null);
    setSelectedCleanCategories(new Set());
    setSelectedMaintenanceTasks(new Set());
    setCompletedCleanCategories(new Set());
    setCompletedMaintenanceTasks(new Set());

    let found = 0;
    let stopLines: (() => void) | undefined;
    try {
      stopLines = await listen<string>("scan-line", (event) => {
        if (event.payload.endsWith(" FOUND")) found += 1;
      });
      const securityWait = await prepareEvent("scan-done");
      const analysisPromise = invoke<SmartScanResult>("run_smart_scan");
      await invoke("start_smart_security_scan");
      const [analysis, scanCode] = await Promise.all([analysisPromise, securityWait.promise]);
      const cleanCategories = analysis.clean_categories.filter(
        (category) =>
          category.size_mb > 0 && category.id !== "trash" && category.id !== "ios_backups"
      );
      setResult(analysis);
      setSelectedCleanCategories(new Set(cleanCategories.map((category) => category.id)));
      setSelectedMaintenanceTasks(new Set(analysis.maintenance_recommendations));
      setThreats(found);
      setSecurityDone(scanCode === 0 || scanCode === 1);
      setStage("ready");
    } catch (reason) {
      setError(String(reason));
      setStage("idle");
    } finally {
      stopLines?.();
    }
  };

  const fix = async () => {
    if (!result || selectedActionCount === 0) return;
    setStage("fixing");
    setError(null);
    setNotice(null);
    const cleanIds = remainingCleanCategories
      .map((category) => category.id)
      .filter((id) => selectedCleanCategories.has(id));
    const taskIds = remainingMaintenanceTasks.filter((task) => selectedMaintenanceTasks.has(task));
    const nextCompletedClean = new Set(completedCleanCategories);
    const nextCompletedMaintenance = new Set(completedMaintenanceTasks);
    try {
      if (cleanIds.length > 0) {
        const cleanWait = await prepareEvent("mo-done");
        await invoke("run_clean_selection", {
          categories: cleanIds,
          installerPaths: [],
        });
        const cleanCode = await cleanWait.promise;
        if (cleanCode !== 0) throw new Error("Le nettoyage n'a pas pu être entièrement réalisé");
        cleanIds.forEach((id) => nextCompletedClean.add(id));
        setCompletedCleanCategories(new Set(nextCompletedClean));
        setSelectedCleanCategories((current) => {
          const next = new Set(current);
          cleanIds.forEach((id) => next.delete(id));
          return next;
        });
      }
      if (taskIds.length > 0) {
        const optimizeWait = await prepareEvent("mo-done");
        await invoke("run_optimize_selection", {
          tasks: taskIds,
        });
        const optimizeCode = await optimizeWait.promise;
        if (optimizeCode !== 0) throw new Error("Certaines optimisations ont échoué");
        taskIds.forEach((task) => nextCompletedMaintenance.add(task));
        setCompletedMaintenanceTasks(new Set(nextCompletedMaintenance));
        setSelectedMaintenanceTasks((current) => {
          const next = new Set(current);
          taskIds.forEach((task) => next.delete(task));
          return next;
        });
      }
      const remaining =
        safeCategories.filter((category) => !nextCompletedClean.has(category.id)).length +
        result.maintenance_recommendations.filter((task) => !nextCompletedMaintenance.has(task))
          .length;
      if (remaining > 0) {
        const completed = cleanIds.length + taskIds.length;
        setNotice(
          `${completed} action${completed === 1 ? " exécutée" : "s exécutées"}. Vous pouvez maintenant sélectionner les actions restantes.`
        );
        setStage("ready");
      } else {
        setStage("done");
      }
    } catch (reason) {
      setError(String(reason));
      setStage("ready");
    }
  };

  const cancelScan = async () => {
    await invoke("cancel_clamav_scan");
    setError("Analyse de sécurité annulée — les résultats déjà calculés restent disponibles.");
  };

  return (
    <div className="flex flex-col h-full px-6 pb-5 gap-4 overflow-y-auto">
      <div className="flex items-center gap-3 pt-1">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "var(--accent-dim)", border: "1px solid var(--border)" }}
        >
          <Sparkles size={18} style={{ color: "var(--accent)" }} />
        </div>
        <div>
          <h2 className="text-lg font-bold" style={{ color: "var(--text-1)" }}>
            Smart Scan
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            Nettoyage, sécurité et optimisation en une seule analyse
          </p>
        </div>
      </div>

      <div
        className="card p-6 flex flex-col items-center text-center gap-4"
        style={{ background: "linear-gradient(135deg, var(--bg-card), var(--accent-dim))" }}
      >
        <div
          className="w-20 h-20 rounded-full flex items-center justify-center"
          style={{ background: "var(--bg)", border: "1px solid var(--border)" }}
        >
          {stage === "running" || stage === "fixing" ? (
            <Loader2
              size={32}
              className="smart-scan-spinner"
              aria-hidden="true"
              style={{ color: "var(--accent)" }}
            />
          ) : stage === "done" ? (
            <CheckCircle2 size={34} style={{ color: "var(--success)" }} />
          ) : (
            <Sparkles size={34} style={{ color: "var(--accent)" }} />
          )}
        </div>
        <div>
          <div className="text-xl font-bold" style={{ color: "var(--text-1)" }}>
            {stage === "idle" && "Un diagnostic complet, sans automatisme risqué"}
            {stage === "running" && "Analyse des trois piliers…"}
            {stage === "ready" &&
              (remainingActionCount === 0
                ? "Aucune correction nécessaire"
                : "Choisissez les actions à exécuter")}
            {stage === "fixing" && "Corrections en cours…"}
            {stage === "done" && "Actions sélectionnées terminées"}
          </div>
          <p className="text-xs mt-1 max-w-xl" style={{ color: "var(--text-3)" }}>
            Burrow analyse les éléments récupérables, lance un contrôle ClamAV rapide et recommande
            uniquement les tâches de maintenance adaptées à l’état actuel du Mac.
          </p>
        </div>
        {stage === "idle" || stage === "done" ? (
          <button onClick={scan} className="btn-primary px-7 py-2.5 flex items-center gap-2">
            <Sparkles size={14} /> {stage === "done" ? "Analyser à nouveau" : "Lancer Smart Scan"}
          </button>
        ) : stage === "ready" ? (
          <button
            onClick={fix}
            disabled={selectedActionCount === 0}
            className="btn-primary px-7 py-2.5 flex items-center gap-2"
          >
            <Wrench size={14} />
            {selectedActionCount === 0
              ? "Sélectionnez au moins une action"
              : `Exécuter la sélection (${selectedActionCount})`}
          </button>
        ) : stage === "running" ? (
          <button
            onClick={cancelScan}
            className="px-7 py-2.5 rounded-lg font-semibold flex items-center gap-2"
            style={{
              color: "var(--text-1)",
              background: "var(--bg-card)",
              border: "1px solid var(--border)",
            }}
          >
            <Square size={12} /> Annuler l’analyse
          </button>
        ) : null}
      </div>

      <div className="grid grid-cols-3 gap-3">
        {[
          {
            icon: Trash2,
            title: "Nettoyage",
            color: "var(--success)",
            value: result ? fmtBytes(result.safe_clean_bytes) : "—",
            detail: result
              ? `${safeCategories.length} catégories sûres`
              : "Caches et fichiers temporaires",
            done: result !== null,
          },
          {
            icon: ShieldCheck,
            title: "Sécurité",
            color: "var(--info)",
            value: result
              ? result.clamav_ready
                ? `${threats} menace${threats === 1 ? "" : "s"}`
                : "Indisponible"
              : "—",
            detail: result?.definitions_outdated
              ? "Définitions à actualiser"
              : "Analyse uniquement · aucune action automatique",
            done: securityDone,
          },
          {
            icon: Gauge,
            title: "Optimisation",
            color: "var(--warning)",
            value: result ? `${result.maintenance_recommendations.length} actions` : "—",
            detail: result
              ? `Disque utilisé à ${Math.round(result.disk_used_percent)} %`
              : "Diagnostic système",
            done: result !== null,
          },
        ].map(({ icon: Icon, title, color, value, detail, done }) => (
          <div key={title} className="card p-4">
            <div className="flex items-center justify-between">
              <Icon size={16} style={{ color }} />
              {done ? (
                <CheckCircle2 size={13} style={{ color: "var(--success)" }} />
              ) : (
                <Circle size={13} style={{ color: "var(--text-3)" }} />
              )}
            </div>
            <div className="text-[10px] uppercase tracking-widest mt-3" style={{ color }}>
              {title}
            </div>
            <div className="text-xl font-bold mt-1" style={{ color: "var(--text-1)" }}>
              {value}
            </div>
            <div className="text-[10px] mt-1" style={{ color: "var(--text-3)" }}>
              {detail}
            </div>
          </div>
        ))}
      </div>

      {result && (
        <div className="grid grid-cols-2 gap-3">
          <div className="card p-4">
            <div className="flex items-center justify-between gap-3 mb-3">
              <h3 className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
                Éléments récupérables
              </h3>
              {remainingCleanCategories.length > 0 && stage === "ready" && (
                <button
                  type="button"
                  className="text-[10px] font-semibold"
                  style={{ color: "var(--accent-text)" }}
                  onClick={() =>
                    setSelectedCleanCategories(
                      selectedCleanCount === remainingCleanCategories.length
                        ? new Set()
                        : new Set(remainingCleanCategories.map((category) => category.id))
                    )
                  }
                >
                  {selectedCleanCount === remainingCleanCategories.length
                    ? "Tout désélectionner"
                    : "Tout sélectionner"}
                </button>
              )}
            </div>
            <div className="space-y-2">
              {safeCategories.length === 0 ? (
                <span className="text-xs" style={{ color: "var(--text-3)" }}>
                  Rien à nettoyer
                </span>
              ) : (
                safeCategories.map((category) => {
                  const completed = completedCleanCategories.has(category.id);
                  const selected = selectedCleanCategories.has(category.id);
                  return (
                    <button
                      key={category.id}
                      type="button"
                      role="checkbox"
                      aria-checked={completed || selected}
                      disabled={completed || stage !== "ready"}
                      onClick={() => toggleSelection(category.id, setSelectedCleanCategories)}
                      className="w-full flex items-center justify-between gap-3 text-xs text-left disabled:cursor-default"
                    >
                      <span className="flex items-center gap-2 min-w-0">
                        {completed || selected ? (
                          <CheckSquare2
                            size={14}
                            style={{ color: completed ? "var(--success)" : "var(--accent)" }}
                          />
                        ) : (
                          <Square size={14} style={{ color: "var(--text-3)" }} />
                        )}
                        <span
                          className={completed ? "line-through" : ""}
                          style={{ color: completed ? "var(--text-3)" : "var(--text-2)" }}
                        >
                          {CLEAN_LABELS[category.id] ?? category.id}
                        </span>
                      </span>
                      <span className="font-mono shrink-0" style={{ color: "var(--text-3)" }}>
                        {completed ? "Terminé" : `${category.size_mb} Mo`}
                      </span>
                    </button>
                  );
                })
              )}
            </div>
          </div>
          <div className="card p-4">
            <div className="flex items-center justify-between gap-3 mb-3">
              <h3 className="text-sm font-semibold" style={{ color: "var(--text-1)" }}>
                Maintenance recommandée
              </h3>
              {remainingMaintenanceTasks.length > 0 && stage === "ready" && (
                <button
                  type="button"
                  className="text-[10px] font-semibold"
                  style={{ color: "var(--accent-text)" }}
                  onClick={() =>
                    setSelectedMaintenanceTasks(
                      selectedMaintenanceCount === remainingMaintenanceTasks.length
                        ? new Set()
                        : new Set(remainingMaintenanceTasks)
                    )
                  }
                >
                  {selectedMaintenanceCount === remainingMaintenanceTasks.length
                    ? "Tout désélectionner"
                    : "Tout sélectionner"}
                </button>
              )}
            </div>
            <div className="space-y-2">
              {result.maintenance_recommendations.length === 0 ? (
                <span className="text-xs" style={{ color: "var(--text-3)" }}>
                  Aucune action recommandée
                </span>
              ) : (
                result.maintenance_recommendations.map((task) => {
                  const completed = completedMaintenanceTasks.has(task);
                  const selected = selectedMaintenanceTasks.has(task);
                  return (
                    <button
                      key={task}
                      type="button"
                      role="checkbox"
                      aria-checked={completed || selected}
                      disabled={completed || stage !== "ready"}
                      onClick={() => toggleSelection(task, setSelectedMaintenanceTasks)}
                      className="w-full flex items-center justify-between gap-3 text-xs text-left disabled:cursor-default"
                    >
                      <span className="flex items-center gap-2 min-w-0">
                        {completed || selected ? (
                          <CheckSquare2
                            size={14}
                            style={{ color: completed ? "var(--success)" : "var(--warning)" }}
                          />
                        ) : (
                          <Square size={14} style={{ color: "var(--text-3)" }} />
                        )}
                        <span
                          className={completed ? "line-through" : ""}
                          style={{ color: completed ? "var(--text-3)" : "var(--text-2)" }}
                        >
                          {TASK_LABELS[task] ?? task}
                        </span>
                      </span>
                      {completed && (
                        <span className="font-mono shrink-0" style={{ color: "var(--text-3)" }}>
                          Terminé
                        </span>
                      )}
                    </button>
                  );
                })
              )}
            </div>
          </div>
        </div>
      )}

      {notice && (
        <div
          className="text-xs px-4 py-3 rounded-xl"
          style={{ background: "var(--success-dim)", color: "var(--success-text)" }}
        >
          {notice}
        </div>
      )}

      {error && (
        <div
          className="text-xs px-4 py-3 rounded-xl"
          style={{ background: "var(--danger-soft)", color: "var(--danger)" }}
        >
          {error}
        </div>
      )}
      {stage === "running" && (
        <div className="text-center text-[10px]" style={{ color: "var(--text-3)" }}>
          Vous pouvez laisser cette page ouverte ; aucune suppression n’est réalisée pendant
          l’analyse.
        </div>
      )}
      {stage === "fixing" && (
        <div className="text-center text-[10px]" style={{ color: "var(--text-3)" }}>
          Seules les actions que vous avez sélectionnées sont en cours d’exécution.
        </div>
      )}
    </div>
  );
}
