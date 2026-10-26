import { useEffect, useMemo, useRef, useState } from "react";
import { CircleCheck, FileText, Folder, KeyRound, ListTree, TriangleAlert } from "lucide-react";
import { api, errorMessage, type Leftover, type Program, type RemovalResult } from "../../lib/api";
import { formatBytes, plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { Dialog } from "../../components/Dialog";
import { Badge, Button, Checkbox, cx, Spinner } from "../../components/ui";
import { ProgramIcon } from "./ProgramIcon";

type Step =
  | { kind: "confirm" }
  | { kind: "running" }
  | { kind: "waiting" }
  | { kind: "stillInstalled"; exitCode: number | null }
  | { kind: "scanning" }
  | { kind: "review"; leftovers: Leftover[] }
  | { kind: "removing" }
  | { kind: "done"; results: RemovalResult[]; recycledBytes: number }
  | { kind: "error"; message: string };

const MSI_USER_CANCELLED = 1602;
const WAIT_FOR_HANDOFF_MS = 90_000;

export function UninstallDialog({
  program,
  size,
  onClose,
}: {
  program: Program;
  size: number | null;
  /** `changed` is true when the program list should be reloaded. */
  onClose: (changed: boolean) => void;
}) {
  const { addReclaimed, toast } = useApp();
  const [step, setStep] = useState<Step>({ kind: "confirm" });
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const changed = useRef(false);
  const cancelled = useRef(false);
  // Bumped whenever the flow moves on, which stops any in-flight wait loop.
  const waitToken = useRef(0);

  useEffect(() => () => void (cancelled.current = true), []);

  const markUninstalled = () => {
    if (!changed.current) {
      changed.current = true;
      if (size) addReclaimed(size);
    }
  };

  const scanLeftovers = async () => {
    waitToken.current++;
    setStep({ kind: "scanning" });
    try {
      const leftovers = await api.scanLeftovers({
        programId: program.id,
        name: program.name,
        publisher: program.publisher,
        installLocation: program.installLocation,
        inferredLocation: program.inferredLocation !== program.installLocation ? program.inferredLocation : null,
      });
      setSelected(new Set(leftovers.flatMap((l, i) => (l.confidence === "high" ? [i] : []))));
      setStep({ kind: "review", leftovers });
    } catch (e) {
      setStep({ kind: "error", message: errorMessage(e) });
    }
  };

  const runUninstaller = async () => {
    setStep({ kind: "running" });
    try {
      const outcome = await api.runUninstaller(program.id);
      if (!outcome.stillInstalled) {
        markUninstalled();
        return scanLeftovers();
      }
      if (outcome.exitCode === MSI_USER_CANCELLED) {
        return setStep({ kind: "stillInstalled", exitCode: outcome.exitCode });
      }
      // Many uninstallers hand off to a copy of themselves and exit right
      // away. Watch the registry entry until it disappears.
      setStep({ kind: "waiting" });
      const token = ++waitToken.current;
      const waiting = () => !cancelled.current && waitToken.current === token;
      const started = Date.now();
      while (waiting() && Date.now() - started < WAIT_FOR_HANDOFF_MS) {
        await new Promise((r) => setTimeout(r, 1500));
        if (waiting() && !(await api.programExists(program.id))) {
          markUninstalled();
          return scanLeftovers();
        }
      }
      if (waiting()) setStep({ kind: "stillInstalled", exitCode: outcome.exitCode });
    } catch (e) {
      setStep({ kind: "error", message: errorMessage(e) });
    }
  };

  const removeSelected = async (leftovers: Leftover[]) => {
    const items = leftovers.filter((_, i) => selected.has(i));
    setStep({ kind: "removing" });
    try {
      const results = await api.removeLeftovers(items);
      changed.current = true;
      const recycledBytes = items
        .filter((item, i) => results[i]?.ok && (item.kind === "folder" || item.kind === "file"))
        .reduce((sum, item) => sum + item.sizeBytes, 0);
      setStep({ kind: "done", results, recycledBytes });
    } catch (e) {
      setStep({ kind: "error", message: errorMessage(e) });
    }
  };

  const close = () => onClose(changed.current);
  const busy = step.kind === "running" || step.kind === "scanning" || step.kind === "removing";
  const title = (
    <span className="flex items-center gap-3">
      <ProgramIcon id={program.id} name={program.name} size={22} />
      <span className="truncate">Uninstall {program.name}</span>
    </span>
  );

  if (step.kind === "review") {
    return (
      <LeftoverReview
        title={title}
        program={program}
        leftovers={step.leftovers}
        selected={selected}
        setSelected={setSelected}
        onSkip={close}
        onRemove={() => removeSelected(step.leftovers)}
      />
    );
  }

  return (
    <Dialog
      title={title}
      onClose={busy ? undefined : close}
      footer={<Footer step={step} onClose={close} onUninstall={runUninstaller} onScan={scanLeftovers} />}
    >
      {step.kind === "confirm" && (
        <div className="flex flex-col gap-3">
          <p>
            Reclaim runs {program.name}'s own uninstaller first. When it finishes, Reclaim looks for files and registry entries it left
            behind so you can choose what to remove.
          </p>
          {size !== null && (
            <p className="text-ink-3">
              This frees about <span className="num text-ink-2">{formatBytes(size)}</span>.
            </p>
          )}
        </div>
      )}
      {(step.kind === "running" || step.kind === "waiting") && (
        <Progress
          label={step.kind === "running" ? "Waiting for the uninstaller" : "The uninstaller is finishing up"}
          detail={
            step.kind === "running"
              ? `Follow the steps in ${program.name}'s uninstaller window. If Windows asks for permission, approve it to continue.`
              : `Reclaim continues as soon as ${program.name} is gone from the installed apps list.`
          }
        />
      )}
      {step.kind === "scanning" && (
        <Progress label="Looking for leftovers" detail="Checking program folders, app data, shortcuts, and the registry." />
      )}
      {step.kind === "removing" && (
        <Progress label="Removing leftovers" detail="Files go to the Recycle Bin. Registry entries are backed up before they're removed." />
      )}
      {step.kind === "stillInstalled" && (
        <div className="flex gap-3">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warn" />
          <p>
            {program.name} is still installed. The uninstaller was cancelled or didn't finish
            {step.exitCode !== null && step.exitCode !== 0 ? ` (exit code ${step.exitCode})` : ""}. You can still scan for leftovers, but
            the app's own files will show up as long as it's installed.
          </p>
        </div>
      )}
      {step.kind === "done" && <DoneSummary step={step} onEmptyBin={() => toast("Open Cleanup and select Recycle Bin to free this space")} />}
      {step.kind === "error" && (
        <div className="flex gap-3">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-danger" />
          <p className="break-words">{step.message}</p>
        </div>
      )}
    </Dialog>
  );
}

function Footer({
  step,
  onClose,
  onUninstall,
  onScan,
}: {
  step: Step;
  onClose: () => void;
  onUninstall: () => void;
  onScan: () => void;
}) {
  switch (step.kind) {
    case "confirm":
      return (
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={onUninstall} data-autofocus>
            Uninstall
          </Button>
        </>
      );
    case "waiting":
      return (
        <Button variant="ghost" onClick={onScan}>
          It's done, scan for leftovers
        </Button>
      );
    case "stillInstalled":
      return (
        <>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          <Button onClick={onScan}>Scan for leftovers anyway</Button>
        </>
      );
    case "done":
    case "error":
      return (
        <Button variant="primary" onClick={onClose} data-autofocus>
          Done
        </Button>
      );
    default:
      return null;
  }
}

function Progress({ label, detail }: { label: string; detail: string }) {
  return (
    <div className="flex items-start gap-3 py-1">
      <Spinner className="mt-0.5 size-4 text-uv-300" />
      <div>
        <div className="font-medium text-ink">{label}</div>
        <div className="mt-1">{detail}</div>
      </div>
    </div>
  );
}

function DoneSummary({ step, onEmptyBin }: { step: Extract<Step, { kind: "done" }>; onEmptyBin: () => void }) {
  const failed = step.results.filter((r) => !r.ok);
  const removed = step.results.length - failed.length;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start gap-3">
        <CircleCheck className="mt-0.5 size-4 shrink-0 text-good" />
        <div>
          <div className="font-medium text-ink">Removed {plural(removed, "leftover")}</div>
          <div className="mt-1">
            Registry entries were backed up as .reg files before removal.
            {step.recycledBytes > 0 && (
              <>
                {" "}
                Files went to the Recycle Bin, so their <span className="num text-ink-2">{formatBytes(step.recycledBytes)}</span> is freed once
                you{" "}
                <button className="text-uv-300 hover:underline" onClick={onEmptyBin}>
                  empty it
                </button>
                .
              </>
            )}
          </div>
        </div>
      </div>
      {failed.length > 0 && (
        <div className="rounded-lg border border-line bg-backdrop/50 p-3">
          <div className="mb-2 flex items-center gap-2 font-medium text-ink">
            <TriangleAlert className="size-3.5 text-warn" />
            {plural(failed.length, "item")} couldn't be removed
          </div>
          <ul className="flex flex-col gap-1.5">
            {failed.map((r) => (
              <li key={r.path + (r.valueName ?? "")} className="text-[12px]">
                <div className="num truncate text-ink-2">{r.valueName ? `${r.path} → ${r.valueName}` : r.path}</div>
                <div className="text-ink-3">{r.error}</div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

const KIND_ICON = {
  folder: <Folder />,
  file: <FileText />,
  registryKey: <ListTree />,
  registryValue: <KeyRound />,
};

const CONFIDENCE: Record<Leftover["confidence"], { label: string; tone: "good" | "uv" | "neutral" }> = {
  high: { label: "Certain", tone: "good" },
  medium: { label: "Likely", tone: "uv" },
  low: { label: "Possible", tone: "neutral" },
};

function LeftoverReview({
  title,
  program,
  leftovers,
  selected,
  setSelected,
  onSkip,
  onRemove,
}: {
  title: React.ReactNode;
  program: Program;
  leftovers: Leftover[];
  selected: Set<number>;
  setSelected: (s: Set<number>) => void;
  onSkip: () => void;
  onRemove: () => void;
}) {
  const groups = useMemo(() => {
    const files: number[] = [];
    const registry: number[] = [];
    leftovers.forEach((l, i) => (l.kind === "folder" || l.kind === "file" ? files : registry).push(i));
    return [
      { label: "Files and folders", ids: files },
      { label: "Registry", ids: registry },
    ].filter((g) => g.ids.length > 0);
  }, [leftovers]);

  const selectedBytes = leftovers.reduce((sum, l, i) => sum + (selected.has(i) ? l.sizeBytes : 0), 0);
  const toggle = (i: number, on: boolean) => {
    const next = new Set(selected);
    if (on) next.add(i);
    else next.delete(i);
    setSelected(next);
  };

  if (leftovers.length === 0) {
    return (
      <Dialog
        title={title}
        onClose={onSkip}
        footer={
          <Button variant="primary" onClick={onSkip} data-autofocus>
            Done
          </Button>
        }
      >
        <div className="flex items-start gap-3">
          <CircleCheck className="mt-0.5 size-4 shrink-0 text-good" />
          <p>No leftovers found. {program.name} cleaned up after itself.</p>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={title}
      onClose={onSkip}
      width={720}
      footer={
        <>
          <span className="mr-auto text-[12.5px] text-ink-3">
            {plural(selected.size, "item")} selected
            {selectedBytes > 0 && <span className="num"> · {formatBytes(selectedBytes)}</span>}
          </span>
          <Button variant="ghost" onClick={onSkip}>
            Keep everything
          </Button>
          <Button variant="danger" disabled={selected.size === 0} onClick={onRemove}>
            Remove selected
          </Button>
        </>
      }
    >
      <p className="mb-4">
        {program.name} left {plural(leftovers.length, "item")} behind. Items marked <Badge tone="good">Certain</Badge> are selected for you;
        review the rest before adding them.
      </p>
      <div className="flex flex-col gap-4">
        {groups.map((g) => {
          const all = g.ids.every((i) => selected.has(i));
          const some = g.ids.some((i) => selected.has(i));
          return (
            <section key={g.label} className="overflow-hidden rounded-lg border border-line">
              <header className="flex items-center gap-3 border-b border-line bg-backdrop/40 px-3 py-2">
                <Checkbox
                  label={`Select all ${g.label.toLowerCase()}`}
                  checked={all}
                  indeterminate={!all && some}
                  onChange={(on) => {
                    const next = new Set(selected);
                    g.ids.forEach((i) => (on ? next.add(i) : next.delete(i)));
                    setSelected(next);
                  }}
                />
                <span className="text-[12px] font-semibold text-ink">{g.label}</span>
                <span className="num text-[11px] text-ink-3">{g.ids.length}</span>
              </header>
              <ul>
                {g.ids.map((i) => {
                  const l = leftovers[i];
                  const conf = CONFIDENCE[l.confidence];
                  return (
                    <li
                      key={i}
                      onClick={() => toggle(i, !selected.has(i))}
                      className={cx(
                        "grid cursor-default grid-cols-[16px_16px_minmax(0,1fr)_auto] items-start gap-3 border-b border-line/60 px-3 py-2.5 last:border-b-0",
                        selected.has(i) ? "bg-uv-600/8" : "hover:bg-white/3",
                      )}
                    >
                      <Checkbox label={l.path} checked={selected.has(i)} onChange={(on) => toggle(i, on)} className="mt-0.5" />
                      <span className="mt-0.5 text-ink-3 [&_svg]:size-4">{KIND_ICON[l.kind]}</span>
                      <div className="min-w-0">
                        <div className="num truncate text-[12px] text-ink" title={l.path}>
                          {l.path}
                          {l.valueName && <span className="text-uv-300"> → {l.valueName}</span>}
                        </div>
                        <div className="mt-0.5 flex items-center gap-2 text-[12px] text-ink-3">
                          <Badge tone={conf.tone}>{conf.label}</Badge>
                          {l.reason}
                        </div>
                      </div>
                      <span className="num pt-0.5 text-[12px] text-ink-2">{l.sizeBytes > 0 ? formatBytes(l.sizeBytes) : ""}</span>
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
      </div>
    </Dialog>
  );
}
