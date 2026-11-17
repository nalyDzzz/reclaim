import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, errorMessage, type Drift } from "../lib/api";
import { plural } from "../lib/format";
import { useApp } from "./app";

/** Long enough for the window to settle before PowerShell starts. */
const FIRST_CHECK_DELAY_MS = 3_000;

interface DebloatState {
  /** Changes that came undone; null until the first check finishes. */
  drift: Drift | null;
  driftCount: number;
  /** Goes up whenever a redo changes settings or apps, so lists showing them reload. */
  revision: number;
  redoing: boolean;
  redo: () => Promise<void>;
  keepAsIs: () => Promise<void>;
  /** Drops an item from the drift the user just dealt with by hand. */
  settleTweak: (id: string) => void;
  settleApp: (name: string) => void;
  needsExplorerRestart: boolean;
  setNeedsExplorerRestart: (needed: boolean) => void;
  /** A change Windows only reads at sign-in is waiting. */
  needsSignIn: boolean;
  setNeedsSignIn: (needed: boolean) => void;
}

const DebloatContext = createContext<DebloatState | null>(null);

function describe(drift: Drift): string {
  const parts = [];
  if (drift.tweaks.length) parts.push(`${plural(drift.tweaks.length, "setting")} went back to the Windows default`);
  if (drift.apps.length) parts.push(`${plural(drift.apps.length, "removed app")} came back`);
  return parts.join(", and ");
}

export function DebloatProvider({ children }: { children: ReactNode }) {
  const { toast } = useApp();
  const [drift, setDrift] = useState<Drift | null>(null);
  const [revision, setRevision] = useState(0);
  const [redoing, setRedoing] = useState(false);
  const [needsExplorerRestart, setNeedsExplorerRestart] = useState(false);
  const [needsSignIn, setNeedsSignIn] = useState(false);
  const toastRef = useRef(toast);
  toastRef.current = toast;

  const check = useCallback(async () => {
    try {
      const found = await api.checkDrift();
      setDrift(found);
      return found;
    } catch {
      setDrift({ tweaks: [], apps: [] });
      return null;
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(async () => {
      const found = await check();
      if (found && found.tweaks.length + found.apps.length > 0) {
        toastRef.current("Some of your Debloat changes came undone", {
          detail: `${describe(found)}, likely after a Windows update. Open Debloat to redo them.`,
          tone: "warn",
        });
      }
    }, FIRST_CHECK_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [check]);

  const redo = useCallback(async () => {
    if (!drift) return;
    setRedoing(true);
    const failed: string[] = [];
    const needAdmin = drift.tweaks.filter((t) => t.blocked);
    for (const t of drift.tweaks.filter((t) => !t.blocked)) {
      try {
        await api.setTweak(t.id, true);
        if (t.restartExplorer) setNeedsExplorerRestart(true);
        if (t.signOut) setNeedsSignIn(true);
      } catch (e) {
        failed.push(`${t.name}: ${errorMessage(e)}`);
      }
    }
    for (const a of drift.apps) {
      try {
        await api.removeAppx(a.name, a.packageFullName);
      } catch (e) {
        failed.push(`${a.displayName}: ${errorMessage(e)}`);
      }
    }
    setRevision((r) => r + 1);
    await check();
    setRedoing(false);
    if (failed.length) {
      toast(`Couldn't redo ${plural(failed.length, "change")}`, { detail: failed.join("\n"), tone: "danger" });
    } else if (needAdmin.length) {
      toast(`${needAdmin.length === 1 ? "1 setting still needs" : `${needAdmin.length} settings still need`} administrator rights`, {
        detail: "Restart Reclaim as administrator to redo the rest.",
        tone: "warn",
      });
    } else {
      toast("Your changes are back in place", { tone: "good" });
    }
  }, [drift, check, toast]);

  const keepAsIs = useCallback(async () => {
    if (!drift) return;
    try {
      await api.forgetDrift(
        drift.tweaks.map((t) => t.id),
        drift.apps.map((a) => a.name),
      );
      setDrift({ tweaks: [], apps: [] });
    } catch (e) {
      toast("Couldn't save that", { detail: errorMessage(e), tone: "danger" });
    }
  }, [drift, toast]);

  const settleTweak = useCallback((id: string) => {
    setDrift((d) => d && { ...d, tweaks: d.tweaks.filter((t) => t.id !== id) });
  }, []);

  const settleApp = useCallback((name: string) => {
    setDrift((d) => d && { ...d, apps: d.apps.filter((a) => a.name.toLowerCase() !== name.toLowerCase()) });
  }, []);

  const driftCount = drift ? drift.tweaks.length + drift.apps.length : 0;
  const value = useMemo(
    () => ({
      drift,
      driftCount,
      revision,
      redoing,
      redo,
      keepAsIs,
      settleTweak,
      settleApp,
      needsExplorerRestart,
      setNeedsExplorerRestart,
      needsSignIn,
      setNeedsSignIn,
    }),
    [drift, driftCount, revision, redoing, redo, keepAsIs, settleTweak, settleApp, needsExplorerRestart, needsSignIn],
  );
  return <DebloatContext.Provider value={value}>{children}</DebloatContext.Provider>;
}

export function useDebloat(): DebloatState {
  const ctx = useContext(DebloatContext);
  if (!ctx) throw new Error("useDebloat must be used inside <DebloatProvider>");
  return ctx;
}
