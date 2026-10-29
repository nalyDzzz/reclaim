import { useEffect, useRef, useState } from "react";
import { FolderSearch, HardDrive, Usb } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type Drive, type ScanProgress, type ScanSummary } from "../../lib/api";
import { bytesParts, formatBytes, formatCount } from "../../lib/format";
import { useApp } from "../../state/app";
import { Button, cx, PageHeader } from "../../components/ui";
import { ScanResults } from "./ScanResults";

type Phase =
  | { kind: "pick" }
  | { kind: "scanning"; path: string; progress: ScanProgress | null }
  | { kind: "results"; summary: ScanSummary };

/** A new object per request, so asking for the same folder again rescans it. */
export interface ScanRequest {
  path: string;
}

export function StorageView({ request }: { request: ScanRequest | null }) {
  const { drives, toast, refreshDrives } = useApp();
  const [phase, setPhase] = useState<Phase>({ kind: "pick" });
  const scanSeq = useRef(0);

  const scan = async (path: string) => {
    const seq = ++scanSeq.current;
    setPhase({ kind: "scanning", path, progress: null });
    try {
      const summary = await api.startScan(path, (progress) => {
        if (scanSeq.current === seq) setPhase({ kind: "scanning", path, progress });
      });
      if (scanSeq.current !== seq) return;
      setPhase(summary.cancelled ? { kind: "pick" } : { kind: "results", summary });
    } catch (e) {
      if (scanSeq.current !== seq) return;
      toast("Couldn't scan that location", { detail: errorMessage(e), tone: "danger" });
      setPhase({ kind: "pick" });
    }
  };

  // Effects run twice in development, so remember the request already started.
  const startedRequest = useRef<ScanRequest | null>(null);
  useEffect(() => {
    if (request && request !== startedRequest.current) {
      startedRequest.current = request;
      scan(request.path);
    }
  }, [request]);

  const chooseFolder = async () => {
    const picked = await open({ directory: true, title: "Choose a folder to scan" });
    if (typeof picked === "string") scan(picked);
  };

  if (phase.kind === "scanning") {
    return <ScanningView path={phase.path} progress={phase.progress} onCancel={() => api.cancelScan()} />;
  }
  if (phase.kind === "results") {
    return (
      <ScanResults
        summary={phase.summary}
        onRescan={() => scan(phase.summary.rootPath)}
        onClose={() => {
          refreshDrives();
          setPhase({ kind: "pick" });
        }}
      />
    );
  }

  const scannable = drives.filter((d) => d.kind !== "optical" && d.totalBytes > 0);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <PageHeader
        title="Storage"
        meta="Scan a drive to see exactly which folders and files are using its space."
        actions={
          <Button icon={<FolderSearch className="size-3.5" />} onClick={chooseFolder}>
            Scan a folder
          </Button>
        }
      />
      <div className="flex flex-col gap-3 px-7 pb-8">
        {scannable.map((d) => (
          <DriveRow key={d.letter} drive={d} onScan={() => scan(d.root)} />
        ))}
      </div>
    </div>
  );
}

/** One drive drawn as its capacity: a wide bar with the used figure large. */
function DriveRow({ drive: d, onScan }: { drive: Drive; onScan: () => void }) {
  const used = d.totalBytes - d.freeBytes;
  const fraction = used / d.totalBytes;
  const { value, unit } = bytesParts(used);
  const tight = fraction > 0.9;
  return (
    <div className="group grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-8 gap-y-3 rounded-xl border border-line bg-white/[0.02] px-5 py-4 transition-colors hover:border-line-strong">
      <div className="flex min-w-0 items-center gap-3">
        <span className="text-ink-3 [&_svg]:size-4">{d.kind === "removable" ? <Usb /> : <HardDrive />}</span>
        <span className="truncate text-[13.5px] text-ink">
          {d.label || (d.kind === "removable" ? "USB drive" : "Local Disk")} <span className="num text-ink-2">({d.letter}:)</span>
        </span>
        <span className="text-[12px] text-ink-3">{d.fileSystem}</span>
      </div>
      <Button variant={tight ? "primary" : "secondary"} onClick={onScan} className="row-span-2 self-center">
        Scan {d.letter}:
      </Button>
      <div className="flex min-w-0 items-end gap-5">
        <div className="w-[150px] shrink-0">
          <span className="display text-[30px] leading-none text-ink">{value}</span>
          <span className="ml-1.5 text-[13px] text-ink-2">{unit} used</span>
        </div>
        <div className="min-w-0 flex-1 pb-1">
          <div className="relative h-2.5 overflow-hidden rounded-full bg-white/7">
            <div
              className={cx("absolute inset-y-0 left-0 rounded-full", fraction > 0.95 ? "bg-danger" : tight ? "bg-warn" : "bg-uv-500")}
              style={{ width: `${fraction * 100}%` }}
            />
          </div>
          <div className="num mt-1.5 flex justify-between text-[11.5px] text-ink-3">
            <span>{Math.round(fraction * 100)}% full</span>
            <span>
              {formatBytes(d.freeBytes)} free of {formatBytes(d.totalBytes)}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

function ScanningView({ path, progress, onCancel }: { path: string; progress: ScanProgress | null; onCancel: () => void }) {
  const { value, unit } = bytesParts(progress?.bytes ?? 0);
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-8 px-10">
      <div className="text-center">
        <div className="text-[12px] font-semibold tracking-[0.14em] text-ink-3 uppercase">Scanning {path}</div>
        <div className="mt-4 flex items-baseline justify-center gap-2">
          <span className="display text-[64px] tabular-nums leading-none text-ink">{value}</span>
          <span className="text-[18px] text-ink-2">{unit}</span>
        </div>
        <div className="num mt-3 text-[13px] text-ink-2">
          {formatCount(progress?.files ?? 0)} files in {formatCount(progress?.dirs ?? 0)} folders
        </div>
      </div>
      <div className="relative h-1 w-full max-w-md overflow-hidden rounded-full bg-white/7">
        <div className="animate-sweep absolute inset-y-0 w-1/3 rounded-full bg-gradient-to-r from-transparent via-uv-400 to-transparent" />
      </div>
      {/* Right-aligned with rtl so the end of a long path stays visible. */}
      <div className="num w-full max-w-xl truncate text-center text-[11.5px] text-ink-3" style={{ direction: "rtl" }} title={progress?.current}>
        <bdi>{progress?.current ?? path}</bdi>
      </div>
      <Button variant="ghost" onClick={onCancel}>
        Cancel scan
      </Button>
    </div>
  );
}
