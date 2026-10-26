import { ExternalLink, FolderOpen, Trash2, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Program } from "../../lib/api";
import { formatBytes, formatDate } from "../../lib/format";
import { Badge, Button, IconButton } from "../../components/ui";
import { ProgramIcon } from "./ProgramIcon";

const SOURCE_LABEL: Record<Program["source"], { scope: string; arch: string }> = {
  machine64: { scope: "Everyone on this PC", arch: "64-bit" },
  machine32: { scope: "Everyone on this PC", arch: "32-bit" },
  user: { scope: "Only you", arch: "Per-user" },
};

export function ProgramDetail({
  program: p,
  size,
  sizeMeasured,
  onClose,
  onUninstall,
}: {
  program: Program;
  size: number | null;
  sizeMeasured: boolean;
  onClose: () => void;
  onUninstall: () => void;
}) {
  const location = p.installLocation ?? p.inferredLocation;
  const source = SOURCE_LABEL[p.source];

  return (
    <aside
      aria-label={`${p.name} details`}
      className="animate-rise flex w-[360px] shrink-0 flex-col border-l border-line bg-backdrop/35"
    >
      <div className="flex items-start gap-3.5 px-5 pt-5">
        <div className="flex size-14 shrink-0 items-center justify-center rounded-xl bg-white/5 ring-1 ring-line">
          <ProgramIcon id={p.id} name={p.name} size={36} />
        </div>
        <div className="min-w-0 flex-1 pt-1">
          <h2 className="text-[16px] leading-snug font-semibold break-words text-ink">{p.name}</h2>
          <div className="mt-0.5 truncate text-[12.5px] text-ink-2">{p.publisher ?? "Unknown publisher"}</div>
        </div>
        <IconButton label="Close details" onClick={onClose} className="-mr-2">
          <X className="size-4" />
        </IconButton>
      </div>

      <div className="flex gap-2 px-5 pt-4 pb-5">
        <Button variant="primary" icon={<Trash2 className="size-3.5" />} onClick={onUninstall} className="flex-1">
          Uninstall
        </Button>
        {location && (
          <Button icon={<FolderOpen className="size-3.5" />} onClick={() => api.openPath(location)}>
            Open folder
          </Button>
        )}
      </div>

      <dl className="min-h-0 flex-1 overflow-y-auto border-t border-line px-5 py-4 text-[12.5px]">
        <Fact label="Size">
          <span className="num text-ink">{size !== null ? formatBytes(size) : "Unknown"}</span>
          {size !== null && <span className="ml-2 text-ink-3">{sizeMeasured ? "measured from its folder" : "reported by the installer"}</span>}
        </Fact>
        <Fact label="Version">
          <span className="num selectable">{p.version ?? "—"}</span>
        </Fact>
        <Fact label="Installed">{formatDate(p.installDate)}</Fact>
        <Fact label="Installed for">
          <span className="flex flex-wrap items-center gap-1.5">
            {source.scope}
            <Badge>{source.arch}</Badge>
            <Badge>{p.isMsi ? "Windows Installer" : "Setup program"}</Badge>
          </span>
        </Fact>
        {location && (
          <Fact label="Location">
            <button
              onClick={() => api.revealPath(location)}
              className="num selectable text-left text-[11.5px] break-all text-ink-2 hover:text-uv-300"
              title="Show in File Explorer"
            >
              {location}
            </button>
          </Fact>
        )}
        {p.url && /^https?:\/\//i.test(p.url) && (
          <Fact label="Website">
            <button onClick={() => openUrl(p.url!)} className="inline-flex items-center gap-1 break-all text-uv-300 hover:underline">
              {p.url.replace(/^https?:\/\//i, "").replace(/\/$/, "")}
              <ExternalLink className="size-3 shrink-0" />
            </button>
          </Fact>
        )}
        {p.comments && <Fact label="Description">{p.comments}</Fact>}
        <Fact label="Uninstall command">
          <code className="num selectable block text-[11.5px] break-all text-ink-2">
            {p.uninstallString ?? (p.isMsi ? "msiexec /x" : "—")}
          </code>
        </Fact>
        <Fact label="Registry key">
          <code className="num selectable block text-[11.5px] break-all text-ink-3">{p.registryPath}</code>
        </Fact>
      </dl>
    </aside>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[104px_minmax(0,1fr)] gap-3 py-2">
      <dt className="text-ink-3">{label}</dt>
      <dd className="min-w-0 text-ink-2">{children}</dd>
    </div>
  );
}
