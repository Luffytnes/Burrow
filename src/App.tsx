import { useState, useEffect, lazy, Suspense } from "react";
import "./App.css";
import Topbar from "./components/Topbar";
import PermissionsGate from "./components/PermissionsGate";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";

const Dashboard = lazy(() => import("./pages/Dashboard"));
const Clean = lazy(() => import("./pages/Clean"));
const Scan = lazy(() => import("./pages/Scan"));
const Optimize = lazy(() => import("./pages/Optimize"));
const Uninstall = lazy(() => import("./pages/Uninstall"));
const Settings = lazy(() => import("./pages/Settings"));
const DnsPage = lazy(() => import("./pages/Dns"));
const ActivityPage = lazy(() => import("./pages/Activity"));
const SmartScan = lazy(() => import("./pages/SmartScan"));

function PageFallback() {
  return (
    <div
      className="flex items-center justify-center h-full"
      style={{ color: "var(--text-3)" }}
      aria-busy="true"
      aria-label="Chargement de la page"
    >
      <div
        className="w-5 h-5 rounded-full border-2 animate-spin"
        style={{ borderColor: "var(--border)", borderTopColor: "var(--accent)" }}
      />
    </div>
  );
}

export default function App() {
  const [active, setActive] = useState("smart-scan");
  // Once Clean is visited once, keep it mounted permanently (avoids re-scan on every navigation)
  const [cleanMounted, setCleanMounted] = useState(active === "clean");
  const reducedMotion = useReducedMotion();

  useEffect(() => {
    if (active === "clean") setCleanMounted(true);
  }, [active]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("navigate", ({ payload }) => setActive(payload)).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, []);

  const renderPage = () => {
    switch (active) {
      case "smart-scan":
        return <SmartScan />;
      case "dashboard":
        return <Dashboard onNavigate={setActive} />;
      case "scan":
        return <Scan />;
      case "optimize":
        return <Optimize />;
      case "uninstall":
        return <Uninstall />;
      case "settings":
        return <Settings />;
      case "dns":
        return <DnsPage />;
      case "activity":
        return <ActivityPage />;
      default:
        return <SmartScan />;
    }
  };

  return (
    <div className="flex flex-col h-screen" style={{ background: "var(--bg)" }}>
      <Topbar active={active} onSelect={setActive} />
      <main className="flex-1 overflow-hidden relative">
        <PermissionsGate>
          {/* Clean reste monté une fois visité — navigation instantanée au retour */}
          {cleanMounted && (
            <div
              className="absolute inset-0"
              style={{
                opacity: active === "clean" ? 1 : 0,
                pointerEvents: active === "clean" ? "auto" : "none",
                transition: "opacity 0.16s ease-out",
                zIndex: active === "clean" ? 1 : 0,
              }}
            >
              <Suspense fallback={<PageFallback />}>
                <Clean />
              </Suspense>
            </div>
          )}

          {/* Toutes les autres pages avec animation */}
          <AnimatePresence mode="wait">
            {active !== "clean" && (
              <motion.div
                key={active}
                initial={reducedMotion ? { opacity: 0 } : { opacity: 0, y: 8 }}
                animate={reducedMotion ? { opacity: 1 } : { opacity: 1, y: 0 }}
                exit={reducedMotion ? { opacity: 0 } : { opacity: 0, y: -6 }}
                transition={{ duration: reducedMotion ? 0.05 : 0.16, ease: "easeOut" }}
                className="h-full"
              >
                <Suspense fallback={<PageFallback />}>{renderPage()}</Suspense>
              </motion.div>
            )}
          </AnimatePresence>
        </PermissionsGate>
      </main>
    </div>
  );
}
