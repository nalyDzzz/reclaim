import { api, errorMessage } from "../../lib/api";
import type { useApp } from "../../state/app";

/** Restarts Reclaim elevated, or says why it's still running without. */
export function relaunchAsAdmin(toast: ReturnType<typeof useApp>["toast"]) {
  api.relaunchAsAdmin().catch((e) => toast("Reclaim is still running without admin rights", { detail: errorMessage(e), tone: "warn" }));
}
