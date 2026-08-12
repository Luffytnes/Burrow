import { useState, useCallback } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type MoStatus = "idle" | "running" | "done" | "error";

export interface MoResult {
  status: MoStatus;
  output: string[];
  error: string | null;
  uninstall: (name: string, appPath: string) => Promise<number>;
  runCmd: (cmd: string, args?: Record<string, unknown>) => Promise<number>;
  reset: () => void;
}

function listenMoEvents(
  setOutput: React.Dispatch<React.SetStateAction<string[]>>,
  setError: React.Dispatch<React.SetStateAction<string | null>>,
  setStatus: React.Dispatch<React.SetStateAction<MoStatus>>,
  resolve: (code: number) => void
): Promise<() => void> {
  return (async () => {
    const unlisteners: Array<() => void> = [];
    const cleanup = () => unlisteners.forEach((u) => u());

    unlisteners.push(
      await listen<string>("mo-output", (e) => {
        setOutput((prev) => [...prev, e.payload]);
      })
    );
    unlisteners.push(
      await listen<number>("mo-done", (e) => {
        cleanup();
        const code = e.payload ?? 0;
        setStatus(code === 0 ? "done" : "error");
        resolve(code);
      })
    );
    unlisteners.push(
      await listen<string>("mo-error", (e) => {
        cleanup();
        setError(e.payload);
        setStatus("error");
        resolve(1);
      })
    );
    return cleanup;
  })();
}

export function useMo(): MoResult {
  const [status, setStatus] = useState<MoStatus>("idle");
  const [output, setOutput] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  const uninstall = useCallback((name: string, appPath: string): Promise<number> => {
    setStatus("running");
    setOutput([]);
    setError(null);

    return new Promise<number>((resolve) => {
      void (async () => {
        let cleanup: (() => void) | undefined;
        try {
          cleanup = await listenMoEvents(setOutput, setError, setStatus, resolve);
          await invoke("uninstall_app", { name, appPath });
        } catch (e) {
          cleanup?.();
          setError(String(e));
          setStatus("error");
          resolve(1);
        }
      })();
    });
  }, []);

  const runCmd = useCallback((cmd: string, args?: Record<string, unknown>): Promise<number> => {
    setStatus("running");
    setOutput([]);
    setError(null);

    return new Promise<number>((resolve) => {
      void (async () => {
        const unlisteners: Array<() => void> = [];
        const cleanup = () => unlisteners.forEach((u) => u());

        try {
          unlisteners.push(
            await listen<string>("mo-output", (e) => {
              setOutput((prev) => [...prev, e.payload]);
            })
          );

          unlisteners.push(
            await listen<number>("mo-done", (e) => {
              cleanup();
              const code = e.payload ?? 0;
              setStatus(code === 0 ? "done" : "error");
              resolve(code);
            })
          );

          unlisteners.push(
            await listen<string>("mo-error", (e) => {
              cleanup();
              setError(e.payload);
              setStatus("error");
              resolve(1);
            })
          );

          await invoke(cmd, args ?? {});
        } catch (e) {
          cleanup();
          setError(String(e));
          setStatus("error");
          resolve(1);
        }
      })();
    });
  }, []);

  const reset = useCallback(() => {
    setStatus("idle");
    setOutput([]);
    setError(null);
  }, []);

  return { status, output, error, uninstall, runCmd, reset };
}

export async function checkMoInstalled(): Promise<boolean> {
  try {
    await invoke<string>("get_mo_path");
    return true;
  } catch {
    return false;
  }
}
