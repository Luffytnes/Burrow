import { useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { Loader2, Terminal, X, CheckCircle2, AlertCircle } from "lucide-react";

function lineColor(line: string): string {
  const l = line.trim();
  if (/^(✓|✔|→ .*terminé|.*success|.*succès)/i.test(l)) return "#4ade80";
  if (/^(✗|error:|erreur|failed|échec)/i.test(l)) return "#f87171";
  if (/^(→|==>|▶)/.test(l)) return "#60a5fa";
  if (/warning|avertissement/i.test(l)) return "#fbbf24";
  if (/^\s*#/.test(l)) return "#6b7280";
  return "#c9d1d9";
}

interface TerminalPanelProps {
  title: string;
  log: string[];
  running: boolean;
  error?: boolean;
  onClose?: () => void;
  /** Tailwind/CSS class pour positionner le panneau */
  className?: string;
  /** Durée de la barre de progression (secondes) */
  progressDuration?: number;
}

export default function TerminalPanel({
  title,
  log,
  running,
  error,
  onClose,
  className = "absolute inset-x-0 bottom-0",
  progressDuration = 8,
}: TerminalPanelProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const isDone = !running;

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" as ScrollBehavior });
  }, [log.length]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 8 }}
      transition={{ duration: 0.2 }}
      className={`overflow-hidden rounded-xl ${className}`}
      style={{ border: "1px solid var(--term-border)" }}
    >
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: "1px solid var(--term-border)", background: "var(--term-surface)" }}
      >
        <div className="flex items-center gap-2">
          {running ? (
            <Loader2 size={12} className="animate-spin" style={{ color: "#60a5fa" }} />
          ) : error ? (
            <AlertCircle size={12} style={{ color: "#f87171" }} />
          ) : (
            <CheckCircle2 size={12} style={{ color: "#4ade80" }} />
          )}
          <Terminal size={11} style={{ color: "#6b7280" }} />
          <span className="text-[11px] font-mono" style={{ color: "var(--term-muted)" }}>
            {title}
          </span>
        </div>
        {isDone && onClose && (
          <button onClick={onClose} style={{ color: "#6b7280" }}>
            <X size={13} />
          </button>
        )}
      </div>

      {/* Log */}
      <div
        className="p-3 h-28 overflow-y-auto font-mono text-[11px] leading-relaxed space-y-0.5"
        style={{ background: "var(--term-bg)" }}
      >
        {log.length > 0 ? (
          log.map((line, i) => (
            <div key={i} style={{ color: lineColor(line) }}>
              {line}
            </div>
          ))
        ) : (
          <div style={{ color: "#6b7280" }}>…</div>
        )}
        <div ref={bottomRef} />
      </div>

      {/* Progress bar */}
      {running && (
        <div className="h-0.5" style={{ background: "var(--term-border)" }}>
          <motion.div
            className="h-full"
            style={{ background: "#2563eb", transformOrigin: "left" }}
            animate={{ scaleX: [0, 1] }}
            transition={{ duration: progressDuration, ease: "linear", repeat: Infinity }}
          />
        </div>
      )}
    </motion.div>
  );
}
