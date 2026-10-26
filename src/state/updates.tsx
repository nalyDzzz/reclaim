import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { errorMessage } from "../lib/api";
import { useApp } from "./app";

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; downloaded: number; total: number | null }
  | { kind: "installing"; update: Update }
  | { kind: "failed"; update: Update | null; message: string };

interface UpdatesState {
  status: UpdateStatus;
  /** The newer version, once one has been found. */
  available: Update | null;
  /** Portable copies can't replace themselves; they link to the download instead. */
  canInstall: boolean;
  checkNow: () => Promise<void>;
  install: () => Promise<void>;
  dialogOpen: boolean;
  openDialog: () => void;
  closeDialog: () => void;
}

const FIRST_CHECK_DELAY_MS = 5_000;
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

const UpdatesContext = createContext<UpdatesState | null>(null);

export function UpdatesProvider({ children }: { children: ReactNode }) {
  const { system, toast } = useApp();
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [available, setAvailable] = useState<Update | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const busy = useRef(false);

  const runCheck = useCallback(
    async (manual: boolean) => {
      if (busy.current) return;
      busy.current = true;
      setStatus({ kind: "checking" });
      try {
        const update = await check({ timeout: 20_000 });
        if (update) {
          setAvailable(update);
          setStatus({ kind: "available", update });
          if (manual) setDialogOpen(true);
        } else {
          setStatus({ kind: "upToDate" });
          if (manual) toast("You're on the latest version", { tone: "good" });
        }
      } catch (e) {
        // Background checks stay quiet; being offline isn't worth an alert.
        setStatus(available ? { kind: "available", update: available } : { kind: "idle" });
        if (manual) toast("Couldn't check for updates", { detail: errorMessage(e), tone: "danger" });
      } finally {
        busy.current = false;
      }
    },
    [available, toast],
  );

  const runCheckRef = useRef(runCheck);
  runCheckRef.current = runCheck;

  useEffect(() => {
    const first = window.setTimeout(() => runCheckRef.current(false), FIRST_CHECK_DELAY_MS);
    const interval = window.setInterval(() => runCheckRef.current(false), CHECK_INTERVAL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(interval);
    };
  }, []);

  const install = useCallback(async () => {
    const update = available;
    if (!update) return;
    let downloaded = 0;
    let total: number | null = null;
    setStatus({ kind: "downloading", update, downloaded, total });
    try {
      // On Windows this launches the installer and exits Reclaim; the
      // installer reopens it once the new version is in place.
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
        } else {
          setStatus({ kind: "installing", update });
          return;
        }
        setStatus({ kind: "downloading", update, downloaded, total });
      });
      setStatus({ kind: "installing", update });
    } catch (e) {
      setStatus({ kind: "failed", update, message: errorMessage(e) });
    }
  }, [available]);

  const value = useMemo<UpdatesState>(
    () => ({
      status,
      available,
      canInstall: !!system && system.installKind !== "portable",
      checkNow: () => runCheck(true),
      install,
      dialogOpen,
      openDialog: () => setDialogOpen(true),
      closeDialog: () => setDialogOpen(false),
    }),
    [status, available, system, runCheck, install, dialogOpen],
  );
  return <UpdatesContext.Provider value={value}>{children}</UpdatesContext.Provider>;
}

export function useUpdates(): UpdatesState {
  const ctx = useContext(UpdatesContext);
  if (!ctx) throw new Error("useUpdates must be used inside <UpdatesProvider>");
  return ctx;
}
