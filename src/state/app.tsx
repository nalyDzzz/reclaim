import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, type Drive, type SystemInfo } from "../lib/api";

export type ToastTone = "neutral" | "good" | "warn" | "danger";

export interface Toast {
  id: number;
  message: string;
  detail?: string;
  tone: ToastTone;
}

interface AppState {
  system: SystemInfo | null;
  drives: Drive[];
  refreshDrives: () => void;
  /** Bytes freed during this session, across every tool. */
  reclaimed: number;
  addReclaimed: (bytes: number) => void;
  toasts: Toast[];
  toast: (message: string, opts?: { detail?: string; tone?: ToastTone }) => void;
  dismissToast: (id: number) => void;
}

const AppContext = createContext<AppState | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [system, setSystem] = useState<SystemInfo | null>(null);
  const [drives, setDrives] = useState<Drive[]>([]);
  const [reclaimed, setReclaimed] = useState(0);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextToast = useRef(1);

  const refreshDrives = useCallback(() => {
    api.listDrives().then(setDrives).catch(() => {});
  }, []);

  useEffect(() => {
    api
      .systemInfo()
      .then((info) => {
        setSystem(info);
        document.documentElement.classList.toggle("mica", info.mica);
      })
      .catch(() => {});
    refreshDrives();
  }, [refreshDrives]);

  const dismissToast = useCallback((id: number) => {
    setToasts((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback(
    (message: string, opts?: { detail?: string; tone?: ToastTone }) => {
      const id = nextToast.current++;
      setToasts((list) => [...list.slice(-3), { id, message, detail: opts?.detail, tone: opts?.tone ?? "neutral" }]);
      window.setTimeout(() => dismissToast(id), opts?.tone === "danger" ? 9000 : 5000);
    },
    [dismissToast],
  );

  const addReclaimed = useCallback(
    (bytes: number) => {
      if (bytes <= 0) return;
      setReclaimed((total) => total + bytes);
      refreshDrives();
    },
    [refreshDrives],
  );

  const value = useMemo(
    () => ({ system, drives, refreshDrives, reclaimed, addReclaimed, toasts, toast, dismissToast }),
    [system, drives, refreshDrives, reclaimed, addReclaimed, toasts, toast, dismissToast],
  );
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppState {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used inside <AppProvider>");
  return ctx;
}
