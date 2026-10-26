import { ArrowDownToLine, ExternalLink, TriangleAlert } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { formatBytes, formatDate } from "../lib/format";
import { useApp } from "../state/app";
import { useUpdates } from "../state/updates";
import { Dialog } from "./Dialog";
import { Button, Meter } from "./ui";

const RELEASES_URL = "https://github.com/nalyDzzz/reclaim/releases";

export function UpdateDialog() {
  const { system } = useApp();
  const { status, available: update, canInstall, install, dialogOpen, closeDialog } = useUpdates();
  if (!dialogOpen || !update) return null;

  const releaseUrl = `${RELEASES_URL}/tag/v${update.version}`;
  const working = status.kind === "downloading" || status.kind === "installing";
  const notes = update.body?.trim();

  return (
    <Dialog
      title={`Reclaim ${update.version} is available`}
      icon={<ArrowDownToLine className="text-uv-300" />}
      onClose={working ? undefined : closeDialog}
      footer={
        working ? null : canInstall ? (
          <>
            <Button variant="ghost" onClick={closeDialog}>
              Later
            </Button>
            <Button variant="primary" onClick={install} data-autofocus>
              {status.kind === "failed" ? "Try again" : "Install and restart"}
            </Button>
          </>
        ) : (
          <>
            <Button variant="ghost" onClick={closeDialog}>
              Later
            </Button>
            <Button variant="primary" icon={<ExternalLink className="size-3.5" />} onClick={() => openUrl(releaseUrl)} data-autofocus>
              Download from GitHub
            </Button>
          </>
        )
      }
    >
      <p>
        You have <span className="num text-ink">{update.currentVersion}</span>
        {update.date && <> · released {formatDate(update.date.slice(0, 10))}</>}.{" "}
        <button className="inline-flex items-center gap-1 text-uv-300 hover:underline" onClick={() => openUrl(releaseUrl)}>
          Full release notes
          <ExternalLink className="size-3" />
        </button>
      </p>

      {notes && (
        <div className="selectable mt-3 max-h-56 overflow-y-auto rounded-lg border border-line bg-backdrop/50 px-3.5 py-3 text-[12.5px] whitespace-pre-wrap text-ink-2">
          {notes}
        </div>
      )}

      {system?.installKind === "portable" && (
        <p className="mt-3 text-ink-3">This is the portable version, which can't replace itself. Download the new one from GitHub.</p>
      )}

      {status.kind === "downloading" && (
        <div className="mt-4">
          <Meter fraction={status.total ? status.downloaded / status.total : 0} />
          <div className="num mt-1.5 text-[12px] text-ink-3">
            Downloading {formatBytes(status.downloaded)}
            {status.total ? ` of ${formatBytes(status.total)}` : ""}
          </div>
        </div>
      )}
      {status.kind === "installing" && (
        <p className="mt-4 text-ink">Installing. Reclaim will close and reopen in a moment.</p>
      )}
      {status.kind === "failed" && (
        <div className="mt-4 flex gap-2 rounded-md bg-danger/10 px-3 py-2 text-[12.5px] text-ink">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-danger" />
          <span className="break-words">The update didn't install: {status.message}</span>
        </div>
      )}
    </Dialog>
  );
}
