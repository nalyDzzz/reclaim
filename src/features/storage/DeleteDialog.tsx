import { useState } from "react";
import { Trash2, TriangleAlert } from "lucide-react";
import { api, errorMessage } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { useApp } from "../../state/app";
import { Dialog } from "../../components/Dialog";
import { Button } from "../../components/ui";

export interface DeleteTarget {
  id: number;
  name: string;
  size: number;
  isDir: boolean;
}

export function DeleteDialog({ target, onClose }: { target: DeleteTarget; onClose: (deleted: boolean) => void }) {
  const { addReclaimed, toast } = useApp();
  const [busy, setBusy] = useState<"recycle" | "permanent" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (permanent: boolean) => {
    setBusy(permanent ? "permanent" : "recycle");
    setError(null);
    try {
      const freed = await api.deleteScanItem(target.id, permanent);
      if (permanent) {
        addReclaimed(freed);
        toast(`Deleted ${target.name}`, { detail: `Freed ${formatBytes(freed)}`, tone: "good" });
      } else {
        toast(`Moved ${target.name} to the Recycle Bin`, {
          detail: "Empty the Recycle Bin from Cleanup to free the space.",
        });
      }
      onClose(true);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(null);
    }
  };

  return (
    <Dialog
      title={`Delete ${target.isDir ? "folder" : "file"} “${target.name}”?`}
      icon={<Trash2 className="text-danger" />}
      onClose={busy ? undefined : () => onClose(false)}
      footer={
        <>
          <Button variant="ghost" onClick={() => onClose(false)} disabled={!!busy} className="mr-auto">
            Cancel
          </Button>
          <Button onClick={() => run(false)} busy={busy === "recycle"} disabled={!!busy} data-autofocus>
            Move to Recycle Bin
          </Button>
          <Button variant="danger" onClick={() => run(true)} busy={busy === "permanent"} disabled={!!busy}>
            Delete permanently
          </Button>
        </>
      }
    >
      <p>
        This {target.isDir ? "folder and everything in it uses" : "file uses"}{" "}
        <span className="num text-ink">{formatBytes(target.size)}</span>. Items in the Recycle Bin still take up space on this drive until
        it's emptied. Permanent deletion can't be undone.
      </p>
      {error && (
        <div className="mt-3 flex gap-2 rounded-md bg-danger/10 px-3 py-2 text-[12.5px] text-ink">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-danger" />
          <span className="break-words">{error}</span>
        </div>
      )}
    </Dialog>
  );
}
