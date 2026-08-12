import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  ShieldAlert,
  Shield,
  ShieldCheck,
  Play,
  Square,
  Folder,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  RotateCcw,
  Trash2,
  Loader2,
  Zap,
  HardDrive,
  FolderOpen,
  Download,
  X,
  Usb,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useT } from "../i18n/useT";

interface ClamavInfo {
  installed: boolean;
  version: string;
  freshclam_path: string;
  has_database: boolean;
  db_path: string;
  db_version: string;
}

interface VolumeInfo {
  name: string;
  path: string;
  total_gb: number;
  free_gb: number;
}

interface QuarantineEntry {
  name: string;
  quarantine_path: string;
  original_path: string;
  size_bytes: number;
  quarantined_at: string;
}

type ScanMode = "quick" | "full" | "custom";
type PageTab = "scanner" | "quarantine";
type TermKind = "ok" | "threat" | "warn" | "dl" | "info";
interface TermLine {
  text: string;
  kind: TermKind;
}

function classify(line: string): TermKind {
  if (line.includes(" FOUND")) return "threat";
  if (line.includes(": OK")) return "ok";
  if (line.includes("✗") || (line.toLowerCase().includes("error") && !line.startsWith("→")))
    return "warn";
  if (
    line.startsWith("→") ||
    line.includes("Downloading") ||
    line.includes("ClamAV update") ||
    line.includes("updated") ||
    line.includes("up-to-date")
  )
    return "dl";
  return "info";
}

function fmtSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${(bytes / 1e3).toFixed(0)} KB`;
}

export default function Scan() {
  const { t } = useT();
  const [tab, setTab] = useState<PageTab>("scanner");
  const [clamavInfo, setClamavInfo] = useState<ClamavInfo | null>(null);
  const [scanMode, setScanMode] = useState<ScanMode>("quick");
  const [customPath, setCustomPath] = useState("");
  const [scanning, setScanning] = useState(false);
  const [updatingDefs, setUpdatingDefs] = useState(false);
  const [scanDone, setScanDone] = useState(false);
  const [termLines, setTermLines] = useState<TermLine[]>([]);
  const [scanProgress, setScanProgress] = useState(0);
  const [toast, setToast] = useState<{ ok: boolean; msg: string; details?: string } | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const threatCountRef = useRef(0);
  const filesCountRef = useRef(0);
  const scanErrorRef = useRef<string | null>(null);
  const [quarantineList, setQuarantineList] = useState<QuarantineEntry[]>([]);
  const [quarantiningPath, setQuarantiningPath] = useState<string | null>(null);
  const [pickingFolder, setPickingFolder] = useState(false);
  const [defsOutdated, setDefsOutdated] = useState<boolean | null>(null);
  const [volumes, setVolumes] = useState<VolumeInfo[]>([]);
  const progressTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const addLine = useCallback((text: string) => {
    const kind = classify(text);
    if (kind === "threat") threatCountRef.current++;
    if (kind === "ok" || kind === "threat") filesCountRef.current++;
    setTermLines((prev) => [...prev, { text, kind }]);
  }, []);

  const threats = useMemo(
    () => termLines.filter((l) => l.kind === "threat").map((l) => l.text.split(":")[0].trim()),
    [termLines]
  );

  const filesScanned = useMemo(
    () => termLines.filter((l) => l.kind === "ok" || l.kind === "threat").length,
    [termLines]
  );

  useEffect(() => {
    invoke<ClamavInfo>("check_clamav")
      .then((info) => {
        setClamavInfo(info);
        if (info.has_database) {
          invoke<boolean>("check_clamav_defs_outdated")
            .then(setDefsOutdated)
            .catch(() => setDefsOutdated(false));
        }
      })
      .catch(() =>
        setClamavInfo({
          installed: false,
          version: "",
          freshclam_path: "",
          has_database: false,
          db_path: "",
          db_version: "",
        })
      );
    const refreshVolumes = () =>
      invoke<VolumeInfo[]>("list_volumes")
        .then(setVolumes)
        .catch((e) => console.error("list_volumes:", e));
    refreshVolumes();
    const volInterval = setInterval(refreshVolumes, 3000);
    return () => clearInterval(volInterval);
  }, []);

  const loadQuarantine = useCallback(() => {
    invoke<QuarantineEntry[]>("list_quarantine")
      .then(setQuarantineList)
      .catch(() => setQuarantineList([]));
  }, []);

  useEffect(() => {
    if (tab === "quarantine") loadQuarantine();
  }, [tab, loadQuarantine]);

  // Progress bar — time-based + file-count boost
  useEffect(() => {
    if (scanning) {
      const increment = scanMode === "quick" ? 1.8 : scanMode === "full" ? 0.25 : 0.9;
      progressTimerRef.current = setInterval(() => {
        setScanProgress((prev) => Math.min(89, prev + increment));
      }, 500);
    } else {
      if (progressTimerRef.current) clearInterval(progressTimerRef.current);
      progressTimerRef.current = null;
    }
    return () => {
      if (progressTimerRef.current) clearInterval(progressTimerRef.current);
    };
  }, [scanning, scanMode]);

  // File-count boost on top of time-based progress
  useEffect(() => {
    if (scanning) {
      const estimated = scanMode === "quick" ? 1000 : scanMode === "full" ? 200000 : 5000;
      const fileProg = Math.min(88, (filesScanned / estimated) * 100);
      setScanProgress((prev) => Math.max(prev, fileProg));
    }
  }, [filesScanned, scanning, scanMode]);

  const getScanPaths = (): string[] => {
    switch (scanMode) {
      case "quick":
        return ["~/Downloads", "~/Desktop", "~/Documents"];
      case "full":
        return ["~"];
      case "custom":
        return customPath ? [customPath] : [];
    }
  };

  const showScanToast = useCallback((threats: number) => {
    const msg =
      threats > 0
        ? `${threats} menace${threats > 1 ? "s" : ""} détectée${threats > 1 ? "s" : ""}`
        : "Analyse terminée, aucune menace";
    setToast({ ok: threats === 0, msg, details: threats > 0 ? undefined : undefined });
    setTimeout(() => setToast(null), threats > 0 ? 7000 : 4000);
  }, []);

  const handleScan = async () => {
    const paths = getScanPaths();
    if (paths.length === 0) return;

    setScanning(true);
    setScanDone(false);
    setScanProgress(0);
    setTermLines([{ text: `→ Analyse : ${paths.join(", ")}`, kind: "dl" }]);
    threatCountRef.current = 0;
    filesCountRef.current = 0;
    scanErrorRef.current = null;

    const unlisteners: Array<() => void> = [];
    const cleanup = () => unlisteners.forEach((u) => u());

    unlisteners.push(await listen<string>("scan-line", (e) => addLine(e.payload)));
    unlisteners.push(
      await listen<string>("scan-error", (e) => {
        scanErrorRef.current = e.payload;
      })
    );
    unlisteners.push(
      await listen<number>("scan-done", (e) => {
        cleanup();
        setScanning(false);
        const succeeded = e.payload === 0 || e.payload === 1;
        setScanDone(succeeded);
        setScanProgress(succeeded ? 100 : 0);
        if (succeeded) {
          showScanToast(threatCountRef.current);
        } else if (e.payload === 130) {
          setToast({ ok: false, msg: "Analyse annulée" });
        } else {
          setToast({
            ok: false,
            msg: "L’analyse n’a pas pu être terminée",
            details: scanErrorRef.current ?? undefined,
          });
        }
      })
    );

    await invoke("start_clamav_scan", { paths });
  };

  const handleCancel = () => {
    invoke("cancel_clamav_scan");
  };

  const handleScanVolume = async (volumePath: string) => {
    setScanMode("custom");
    setCustomPath(volumePath);
    setScanning(true);
    setScanDone(false);
    setScanProgress(0);
    setTermLines([{ text: `→ Analyse : ${volumePath}`, kind: "dl" }]);
    threatCountRef.current = 0;
    filesCountRef.current = 0;
    scanErrorRef.current = null;

    const unlisteners: Array<() => void> = [];
    const cleanup = () => unlisteners.forEach((u) => u());

    unlisteners.push(await listen<string>("scan-line", (e) => addLine(e.payload)));
    unlisteners.push(
      await listen<string>("scan-error", (e) => {
        scanErrorRef.current = e.payload;
      })
    );
    unlisteners.push(
      await listen<number>("scan-done", (e) => {
        cleanup();
        setScanning(false);
        const succeeded = e.payload === 0 || e.payload === 1;
        setScanDone(succeeded);
        setScanProgress(succeeded ? 100 : 0);
        if (succeeded) {
          showScanToast(threatCountRef.current);
        } else if (e.payload === 130) {
          setToast({ ok: false, msg: "Analyse annulée" });
        } else {
          setToast({
            ok: false,
            msg: "L’analyse n’a pas pu être terminée",
            details: scanErrorRef.current ?? undefined,
          });
        }
      })
    );

    await invoke("start_clamav_scan", { paths: [volumePath] });
  };

  const handleUpdateDefs = async () => {
    setUpdatingDefs(true);
    setTermLines([]);

    const unlisteners: Array<() => void> = [];
    const cleanup = () => unlisteners.forEach((u) => u());
    const lines: string[] = [];

    unlisteners.push(
      await listen<string>("clamav-update-line", (e) => {
        lines.push(e.payload);
        addLine(e.payload);
      })
    );
    unlisteners.push(
      await listen<number>("clamav-update-done", (e) => {
        cleanup();
        setUpdatingDefs(false);
        if (e.payload === 0) {
          setToast({ ok: true, msg: "Définitions ClamAV mises à jour" });
          setTimeout(() => setToast(null), 3500);
          invoke<ClamavInfo>("check_clamav")
            .then((info) => {
              setClamavInfo(info);
              if (info.has_database) {
                invoke<boolean>("check_clamav_defs_outdated")
                  .then(setDefsOutdated)
                  .catch(() => setDefsOutdated(false));
              }
            })
            .catch((e) => console.error("check_clamav after update:", e));
        } else {
          setToast({
            ok: false,
            msg: "Échec de la mise à jour des définitions",
            details: lines.join("\n"),
          });
          setTimeout(() => setToast(null), 7000);
        }
      })
    );

    await invoke("update_clamav_defs");
  };

  const handlePickFolder = async () => {
    setPickingFolder(true);
    try {
      const path = await invoke<string | null>("pick_folder");
      if (path) setCustomPath(path);
    } finally {
      setPickingFolder(false);
    }
  };

  const handleQuarantine = async (threatPath: string) => {
    setQuarantiningPath(threatPath);
    try {
      await invoke("quarantine_file", { originalPath: threatPath });
      addLine(`→ Mis en quarantaine : ${threatPath}`);
    } catch (e) {
      addLine(`✗ Quarantaine échouée : ${e}`);
    } finally {
      setQuarantiningPath(null);
    }
  };

  const handleRestore = async (name: string) => {
    try {
      await invoke("restore_from_quarantine", { name });
      loadQuarantine();
    } catch (e) {
      console.error("restore_from_quarantine:", e);
    }
  };

  const handleDeleteFromQuarantine = async (name: string) => {
    try {
      await invoke("delete_from_quarantine", { name });
      loadQuarantine();
    } catch (e) {
      console.error("delete_from_quarantine:", e);
    }
  };

  const scanModes: { id: ScanMode; labelKey: string; descKey: string; icon: typeof Zap }[] = [
    { id: "quick", labelKey: "scan_mode_quick", descKey: "scan_mode_quick_desc", icon: Zap },
    { id: "full", labelKey: "scan_mode_full", descKey: "scan_mode_full_desc", icon: HardDrive },
    {
      id: "custom",
      labelKey: "scan_mode_custom",
      descKey: "scan_mode_custom_desc",
      icon: FolderOpen,
    },
  ];

  const showTerm = termLines.length > 0;

  return (
    <div className="flex flex-col h-full px-6 pb-4 gap-3 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-3 pt-1 shrink-0">
        <div
          className="w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
        >
          <ShieldAlert size={16} style={{ color: "var(--text-3)" }} />
        </div>
        <div>
          <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
            {t.scan_title}
          </h2>
          <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
            {t.scan_sub}
          </p>
        </div>

        {clamavInfo && (
          <div className="ml-auto flex flex-col items-end gap-0.5">
            {(() => {
              const outdated = defsOutdated === true;
              const color = !clamavInfo.installed
                ? "var(--danger)"
                : outdated
                  ? "var(--danger)"
                  : "var(--success)";
              const bg =
                !clamavInfo.installed || outdated ? "rgba(239,68,68,0.08)" : "rgba(34,197,94,0.08)";
              const border =
                !clamavInfo.installed || outdated
                  ? "1px solid rgba(239,68,68,0.25)"
                  : "1px solid rgba(34,197,94,0.25)";
              return (
                <>
                  <div
                    className="flex items-center gap-1.5 text-xs px-3 py-1.5 rounded-full"
                    style={{ background: bg, border, color }}
                  >
                    {clamavInfo.installed ? (
                      <>
                        <ShieldCheck size={11} /> ClamAV {clamavInfo.version.split(" ").pop()}
                      </>
                    ) : (
                      <>
                        <XCircle size={11} /> {t.scan_not_installed}
                      </>
                    )}
                  </div>
                  {clamavInfo.installed && clamavInfo.db_version && (
                    <span className="text-[10px] font-mono pr-1" style={{ color }}>
                      Définition virale n°{clamavInfo.db_version}
                    </span>
                  )}
                </>
              );
            })()}
          </div>
        )}
      </div>

      {/* Loading */}
      {!clamavInfo ? (
        <div
          className="flex items-center justify-center flex-1 gap-2"
          style={{ color: "var(--text-3)" }}
        >
          <Loader2 size={15} className="animate-spin" />
          <span className="text-sm">{t.common_loading}</span>
        </div>
      ) : !clamavInfo.installed ? (
        <div className="flex flex-col gap-3 flex-1 justify-center items-center">
          <div className="card rounded-2xl p-6 text-center max-w-sm">
            <ShieldAlert size={32} className="mx-auto mb-3" style={{ color: "var(--accent)" }} />
            <p className="text-sm font-semibold mb-2" style={{ color: "var(--text-1)" }}>
              {t.scan_not_installed}
            </p>
            <p className="text-[11px] mb-4" style={{ color: "var(--text-3)" }}>
              {t.scan_sub}
            </p>
            <code
              className="block text-xs font-mono px-3 py-2 rounded-lg"
              style={{
                background: "var(--bg)",
                border: "1px solid var(--border)",
                color: "var(--text-2)",
              }}
            >
              {t.scan_install_cmd}
            </code>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-3 flex-1 min-h-0 overflow-hidden">
          {/* No database banner */}
          {!clamavInfo.has_database && (
            <div
              className="card rounded-xl overflow-hidden shrink-0"
              style={{ borderColor: "rgba(234,179,8,0.35)" }}
            >
              <div className="px-4 py-3 flex items-center gap-3">
                <Download size={14} style={{ color: "var(--warning-text)", flexShrink: 0 }} />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                    {t.scan_no_database}
                  </div>
                  <div className="text-[11px]" style={{ color: "var(--warning-text)" }}>
                    {t.scan_db_required}
                  </div>
                </div>
                <button
                  onClick={handleUpdateDefs}
                  disabled={updatingDefs}
                  className="flex items-center gap-1.5 text-xs font-medium px-3 py-1.5 rounded-lg transition-all disabled:opacity-50 shrink-0"
                  style={{
                    background: "rgba(234,179,8,0.12)",
                    color: "var(--warning-text)",
                    border: "1px solid rgba(234,179,8,0.3)",
                  }}
                >
                  {updatingDefs ? (
                    <>
                      <Loader2 size={11} className="animate-spin" /> {t.scan_updating_defs}
                    </>
                  ) : (
                    <>
                      <Download size={11} /> {t.scan_download_db}
                    </>
                  )}
                </button>
              </div>
            </div>
          )}

          {/* Tabs */}
          <div
            className="flex gap-1 p-1 rounded-xl shrink-0"
            style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
          >
            {(["scanner", "quarantine"] as PageTab[]).map((id) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className="flex-1 py-1.5 text-xs font-semibold rounded-lg transition-all"
                style={
                  tab === id
                    ? {
                        background: "var(--accent-dim)",
                        color: "var(--accent-text)",
                        border: "1px solid rgba(255,0,0,0.15)",
                      }
                    : { color: "var(--text-3)", border: "1px solid transparent" }
                }
              >
                {id === "scanner" ? (
                  t.scan_scanner_tab
                ) : (
                  <>
                    {t.scan_quarantine_tab}
                    {quarantineList.length > 0 && (
                      <span
                        className="ml-1.5 inline-block rounded-full text-[10px] px-1.5 py-0.5"
                        style={{ background: "rgba(239,68,68,0.1)", color: "var(--danger)" }}
                      >
                        {quarantineList.length}
                      </span>
                    )}
                  </>
                )}
              </button>
            ))}
          </div>

          <AnimatePresence mode="wait">
            {tab === "scanner" ? (
              <motion.div
                key="scanner"
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.15 }}
                className="flex flex-col gap-3 flex-1 min-h-0 overflow-hidden"
              >
                {/* Detected external volumes */}
                {volumes.length > 0 && (
                  <div className="card rounded-xl overflow-hidden shrink-0">
                    {volumes.map((vol, i) => (
                      <div
                        key={vol.path}
                        style={i > 0 ? { borderTop: "1px solid var(--border)" } : undefined}
                        className="flex items-center gap-3 px-4 py-2.5"
                      >
                        <Usb size={14} style={{ color: "var(--accent)", flexShrink: 0 }} />
                        <div className="flex-1 min-w-0">
                          <div className="text-sm font-medium" style={{ color: "var(--text-1)" }}>
                            {vol.name}
                          </div>
                          {vol.total_gb > 0 && (
                            <div className="text-[10px]" style={{ color: "var(--text-3)" }}>
                              {vol.free_gb.toFixed(1)} Go libre · {vol.total_gb.toFixed(1)} Go
                            </div>
                          )}
                        </div>
                        <button
                          onClick={() => handleScanVolume(vol.path)}
                          disabled={scanning || updatingDefs || !clamavInfo?.has_database}
                          className="flex items-center gap-1.5 text-xs font-medium px-3 py-1.5 rounded-lg disabled:opacity-40 transition-all shrink-0"
                          style={{ background: "var(--accent)", color: "#fff" }}
                        >
                          <Play size={10} fill="white" />
                          Scanner
                        </button>
                      </div>
                    ))}
                  </div>
                )}

                {/* Scan mode selector — always visible */}
                <div className="grid grid-cols-3 gap-2 shrink-0">
                  {scanModes.map(({ id, labelKey, descKey, icon: Icon }) => (
                    <button
                      key={id}
                      onClick={() => {
                        if (!scanning) setScanMode(id);
                      }}
                      className="card rounded-xl px-3 py-3 text-left flex flex-col gap-1.5 transition-all"
                      style={
                        scanMode === id
                          ? { borderColor: "rgba(255,0,0,0.3)", background: "var(--accent-dim)" }
                          : { opacity: scanning ? 0.5 : 1 }
                      }
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className="w-3 h-3 rounded-full border-2 transition-colors"
                          style={
                            scanMode === id
                              ? { borderColor: "var(--accent)", background: "var(--accent-dim)" }
                              : { borderColor: "var(--border)" }
                          }
                        />
                        <Icon
                          size={12}
                          style={{ color: scanMode === id ? "var(--accent)" : "var(--text-3)" }}
                        />
                      </div>
                      <div className="text-xs font-semibold" style={{ color: "var(--text-1)" }}>
                        {t[labelKey]}
                      </div>
                      <div className="text-[10px] leading-tight" style={{ color: "var(--text-3)" }}>
                        {t[descKey]}
                      </div>
                    </button>
                  ))}
                </div>

                {/* Custom path */}
                {scanMode === "custom" && (
                  <div className="flex gap-2 shrink-0">
                    <input
                      value={customPath}
                      onChange={(e) => setCustomPath(e.target.value)}
                      placeholder={t.scan_custom_placeholder}
                      disabled={scanning}
                      className="card rounded-xl px-4 py-2 flex-1 text-sm outline-none disabled:opacity-50"
                      style={{ color: "var(--text-1)", caretColor: "var(--accent)" }}
                    />
                    <button
                      onClick={handlePickFolder}
                      disabled={pickingFolder || scanning}
                      className="card rounded-xl px-3 py-2 transition-colors disabled:opacity-50"
                      style={{ color: "var(--text-3)" }}
                      onMouseEnter={(e) =>
                        ((e.currentTarget as HTMLElement).style.color = "var(--accent)")
                      }
                      onMouseLeave={(e) =>
                        ((e.currentTarget as HTMLElement).style.color = "var(--text-3)")
                      }
                    >
                      {pickingFolder ? (
                        <Loader2 size={14} className="animate-spin" />
                      ) : (
                        <Folder size={14} />
                      )}
                    </button>
                  </div>
                )}

                {/* Barre de statut de l'analyse */}
                {showTerm && (
                  <div
                    className="rounded-xl overflow-hidden shrink-0"
                    style={{ border: "1px solid var(--term-border)" }}
                  >
                    <div
                      className="flex items-center gap-2 px-3 py-2"
                      style={{ background: "var(--term-surface)" }}
                    >
                      {scanning || updatingDefs ? (
                        <Loader2
                          size={10}
                          className="animate-spin"
                          style={{ color: "var(--text-3)", flexShrink: 0 }}
                        />
                      ) : scanDone ? (
                        threats.length > 0 ? (
                          <AlertTriangle
                            size={10}
                            style={{ color: "var(--danger-text)", flexShrink: 0 }}
                          />
                        ) : (
                          <CheckCircle2
                            size={10}
                            style={{ color: "var(--success-text)", flexShrink: 0 }}
                          />
                        )
                      ) : null}
                      <span className="text-[10px] flex-1" style={{ color: "var(--term-muted)" }}>
                        {scanning
                          ? `${t.scan_scanning} · ${filesScanned} fichiers · ${Math.round(scanProgress)}%`
                          : updatingDefs
                            ? t.scan_updating_defs
                            : scanDone
                              ? `${filesScanned} ${t.scan_files_scanned} · ${
                                  threats.length > 0
                                    ? `${threats.length} ${t.scan_threats_found}`
                                    : t.scan_no_threats
                                }`
                              : ""}
                      </span>
                    </div>
                    {scanning && (
                      <div className="h-[3px] w-full" style={{ background: "var(--border)" }}>
                        <motion.div
                          className="h-full"
                          style={{ background: "var(--accent)" }}
                          animate={{ width: `${scanProgress}%` }}
                          transition={{ duration: 0.4, ease: "easeOut" }}
                        />
                      </div>
                    )}
                    {scanDone && scanProgress === 100 && (
                      <motion.div
                        className="h-[3px] w-full"
                        style={{
                          background: threats.length > 0 ? "var(--danger)" : "var(--success)",
                        }}
                        initial={{ opacity: 1 }}
                        animate={{ opacity: 0 }}
                        transition={{ delay: 0.6, duration: 0.4 }}
                      />
                    )}
                  </div>
                )}

                {/* Threats list */}
                {threats.length > 0 && (
                  <div
                    className="card rounded-xl overflow-hidden shrink-0"
                    style={{ borderColor: "rgba(239,68,68,0.25)" }}
                  >
                    {threats.map((path, i) => (
                      <div
                        key={i}
                        className="flex items-center gap-2 px-3 py-2"
                        style={{
                          borderBottom:
                            i < threats.length - 1 ? "1px solid var(--border)" : undefined,
                        }}
                      >
                        <AlertTriangle
                          size={11}
                          style={{ color: "var(--danger)", flexShrink: 0 }}
                        />
                        <span
                          className="flex-1 text-xs font-mono truncate min-w-0"
                          style={{ color: "var(--text-1)" }}
                        >
                          {path.split("/").pop()}
                        </span>
                        <button
                          onClick={() => handleQuarantine(path)}
                          disabled={quarantiningPath === path}
                          className="flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-lg transition-all disabled:opacity-50 shrink-0"
                          style={{
                            background: "rgba(239,68,68,0.08)",
                            color: "var(--danger)",
                            border: "1px solid rgba(239,68,68,0.25)",
                          }}
                        >
                          {quarantiningPath === path ? (
                            <Loader2 size={9} className="animate-spin" />
                          ) : (
                            <Shield size={9} />
                          )}
                          {t.scan_quarantine_btn}
                        </button>
                      </div>
                    ))}
                  </div>
                )}

                {/* Scan / Cancel */}
                <div className="shrink-0 mt-auto">
                  {!scanning ? (
                    <motion.button
                      whileHover={
                        !clamavInfo.has_database || (scanMode === "custom" && !customPath)
                          ? {}
                          : { scale: 1.01 }
                      }
                      whileTap={
                        !clamavInfo.has_database || (scanMode === "custom" && !customPath)
                          ? {}
                          : { scale: 0.98 }
                      }
                      onClick={handleScan}
                      disabled={
                        !clamavInfo.has_database ||
                        updatingDefs ||
                        (scanMode === "custom" && !customPath)
                      }
                      className="w-full flex items-center justify-center gap-2 py-2.5 rounded-xl font-semibold text-sm disabled:opacity-40 transition-opacity"
                      style={{ background: "var(--accent)", color: "#ffffff" }}
                    >
                      <Play size={13} fill="white" />
                      {!clamavInfo.has_database ? t.scan_no_database : t.scan_start}
                    </motion.button>
                  ) : (
                    /* Cancel button with progress bar overlay */
                    <div
                      className="relative rounded-xl overflow-hidden card"
                      style={{ borderColor: "rgba(239,68,68,0.3)" }}
                    >
                      {/* Progress fill background */}
                      <motion.div
                        className="absolute inset-0 origin-left"
                        style={{ background: "rgba(239,68,68,0.08)" }}
                        animate={{ scaleX: scanProgress / 100 }}
                        transition={{ duration: 0.4, ease: "easeOut" }}
                      />
                      <button
                        onClick={handleCancel}
                        className="relative w-full flex items-center justify-center gap-2 py-2.5 font-semibold text-sm"
                        style={{ color: "var(--danger)" }}
                      >
                        <Square size={11} fill="var(--danger)" />
                        {t.scan_cancel} · {Math.round(scanProgress)}%
                      </button>
                    </div>
                  )}
                </div>
              </motion.div>
            ) : (
              /* Quarantine tab */
              <motion.div
                key="quarantine"
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.15 }}
                className="flex flex-col gap-2 flex-1 overflow-y-auto min-h-0"
              >
                {quarantineList.length === 0 ? (
                  <div
                    className="flex flex-col items-center justify-center flex-1 gap-2"
                    style={{ color: "var(--text-3)" }}
                  >
                    <ShieldCheck size={24} />
                    <span className="text-sm">{t.scan_quarantine_empty}</span>
                  </div>
                ) : (
                  quarantineList.map((entry, i) => (
                    <motion.div
                      key={entry.name}
                      initial={{ opacity: 0, x: -6 }}
                      animate={{ opacity: 1, x: 0 }}
                      transition={{ delay: Math.min(i * 0.04, 0.25) }}
                      className="card rounded-xl px-4 py-3 flex items-center gap-3"
                    >
                      <div
                        className="w-8 h-8 rounded-xl flex items-center justify-center shrink-0"
                        style={{
                          background: "rgba(239,68,68,0.08)",
                          border: "1px solid rgba(239,68,68,0.2)",
                        }}
                      >
                        <ShieldAlert size={14} style={{ color: "var(--danger)" }} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div
                          className="text-sm font-medium truncate"
                          style={{ color: "var(--text-1)" }}
                        >
                          {entry.original_path.split("/").pop()}
                        </div>
                        <div className="text-[10px] truncate" style={{ color: "var(--text-3)" }}>
                          {entry.original_path}
                        </div>
                      </div>
                      <span className="text-xs shrink-0" style={{ color: "var(--danger)" }}>
                        {fmtSize(entry.size_bytes)}
                      </span>
                      <button
                        onClick={() => handleRestore(entry.name)}
                        title={t.scan_restore}
                        className="p-1.5 rounded-lg transition-all shrink-0"
                        style={{ color: "var(--text-3)", border: "1px solid var(--border)" }}
                        onMouseEnter={(e) =>
                          ((e.currentTarget as HTMLElement).style.color = "var(--success)")
                        }
                        onMouseLeave={(e) =>
                          ((e.currentTarget as HTMLElement).style.color = "var(--text-3)")
                        }
                      >
                        <RotateCcw size={12} />
                      </button>
                      <button
                        onClick={() => handleDeleteFromQuarantine(entry.name)}
                        title={t.scan_delete_perm}
                        className="p-1.5 rounded-lg transition-all shrink-0"
                        style={{ color: "var(--text-3)", border: "1px solid var(--border)" }}
                        onMouseEnter={(e) =>
                          ((e.currentTarget as HTMLElement).style.color = "var(--danger)")
                        }
                        onMouseLeave={(e) =>
                          ((e.currentTarget as HTMLElement).style.color = "var(--text-3)")
                        }
                      >
                        <Trash2 size={12} />
                      </button>
                    </motion.div>
                  ))
                )}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      )}

      {/* Toast */}
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
            {toast.ok ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
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
