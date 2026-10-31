import { useEffect, useMemo, useRef, useState } from "react";
import { CalendarClock, CircleCheck, Cog, FileText, Folder, KeyRound, ListTree, Shield, TriangleAlert } from "lucide-react";
import { api, errorMessage, type Leftover, type Program } from "../../lib/api";
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
  | { kind: "removing"; permanent: boolean }
  | ({ kind: "done"; failures: Failure[] } & Totals)
  | { kind: "error"; message: string };

/** `tooBig` means only the Recycle Bin refused it, so deleting permanently works. */
type Failure = { item: Leftover; error: string; tooBig: boolean };
type Totals = { removed: number; recycledBytes: number; deletedBytes: number };

const NO_TOTALS: Totals = { removed: 0, recycledBytes: 0, deletedBytes: 0 };

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
  const { addReclaimed, toast, system } = useApp();
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

  /** On a later pass, `before` carries the earlier totals and `kept` the failures this pass leaves alone. */
  const removeItems = async (items: Leftover[], { permanent = false, before = NO_TOTALS, kept = [] as Failure[] } = {}) => {
    setStep({ kind: "removing", permanent });
    try {
      const results = await api.removeLeftovers(items, permanent);
      changed.current = true;
      const failures = items.flatMap((item, i) => {
        const result = results[i];
        return result?.ok ? [] : [{ item, error: result?.error ?? "Unknown error", tooBig: result?.tooBigForRecycleBin ?? false }];
      });
      const freed = items
        .filter((item, i) => results[i]?.ok && (item.kind === "folder" || item.kind === "file"))
        .reduce((sum, item) => sum + item.sizeBytes, 0);
      if (permanent) addReclaimed(freed);
      setStep({
        kind: "done",
        failures: [...kept, ...failures],
        removed: before.removed + items.length - failures.length,
        recycledBytes: before.recycledBytes + (permanent ? 0 : freed),
        deletedBytes: before.deletedBytes + (permanent ? freed : 0),
      });
    } catch (e) {
      setStep({ kind: "error", message: errorMessage(e) });
    }
  };

  /** Retries the failures the Recycle Bin refused (`permanent`) or the rest. */
  const retry = (permanent: boolean) => {
    if (step.kind !== "done") return;
    const again = step.failures.filter((f) => f.tooBig === permanent);
    const kept = step.failures.filter((f) => f.tooBig !== permanent);
    removeItems(again.map((f) => f.item), { permanent, before: step, kept });
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
        onRemove={() => removeItems(step.leftovers.filter((_, i) => selected.has(i)))}
      />
    );
  }

  return (
    <Dialog
      title={title}
      onClose={busy ? undefined : close}
      footer={<Footer step={step} onClose={close} onUninstall={runUninstaller} onScan={scanLeftovers} onRetry={retry} />}
    >
      {step.kind === "confirm" && (
        <div className="flex flex-col gap-3">
          <p>
            Reclaim runs {program.name}'s own uninstaller first. When it finishes, Reclaim looks for files, registry entries, services,
            and scheduled tasks it left behind so you can choose what to remove.
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
        <Progress
          label="Looking for leftovers"
          detail="Checking program folders, app data, shortcuts, the registry, services, scheduled tasks, and firewall rules."
        />
      )}
      {step.kind === "removing" && (
        <Progress
          label={step.permanent ? "Deleting permanently" : "Removing leftovers"}
          detail={
            (step.permanent
              ? "These skip the Recycle Bin."
              : "Files go to the Recycle Bin. Registry entries, services, and tasks are backed up before they're removed.") +
            (system?.elevated ? "" : " If Windows asks for permission, approve it so Reclaim can clear protected places like Program Files.")
          }
        />
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
  onRetry,
}: {
  step: Step;
  onClose: () => void;
  onUninstall: () => void;
  onScan: () => void;
  onRetry: (permanent: boolean) => void;
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
    case "error": {
      // The app is gone from the list now, so this is the last chance to clear what's left.
      const failures = step.kind === "done" ? step.failures : [];
      const tooBigBytes = failures.filter((f) => f.tooBig).reduce((sum, f) => sum + f.item.sizeBytes, 0);
      return (
        <>
          {failures.some((f) => !f.tooBig) && <Button onClick={() => onRetry(false)}>Try again</Button>}
          {failures.some((f) => f.tooBig) && (
            <Button variant="danger" onClick={() => onRetry(true)}>
              {tooBigBytes > 0 ? `Delete ${formatBytes(tooBigBytes)} permanently` : "Delete permanently"}
            </Button>
          )}
          <Button variant="primary" onClick={onClose} data-autofocus>
            Done
          </Button>
        </>
      );
    }
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
  const { removed, failures } = step;
  return (
    <div className="flex flex-col gap-3">
      {removed > 0 ? (
        <div className="flex items-start gap-3">
          <CircleCheck className="mt-0.5 size-4 shrink-0 text-good" />
          <div>
            <div className="font-medium text-ink">Removed {plural(removed, "leftover")}</div>
            <div className="mt-1">
              Registry entries, services, and tasks were backed up before removal.
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
              {step.deletedBytes > 0 && (
                <>
                  {" "}
                  <span className="num text-ink-2">{formatBytes(step.deletedBytes)}</span> was deleted permanently and is already free.
                </>
              )}
            </div>
          </div>
        </div>
      ) : (
        <div className="flex items-start gap-3">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warn" />
          <div className="font-medium text-ink">Nothing was removed</div>
        </div>
      )}
      {failures.length > 0 && (
        <div className="rounded-lg border border-line bg-backdrop/50 p-3">
          <div className="mb-2 flex items-center gap-2 font-medium text-ink">
            <TriangleAlert className="size-3.5 text-warn" />
            {plural(failures.length, "item")} couldn't be removed
          </div>
          {failures.some((f) => f.tooBig) && (
            <p className="mb-2 text-[12px] text-ink-3">
              The Recycle Bin can't take some of these. Deleting them permanently frees the space now, but they can't be restored.
            </p>
          )}
          <ul className="flex flex-col gap-1.5">
            {failures.map(({ item, error }) => (
              <li key={item.kind + item.path + (item.valueName ?? "")} className="text-[12px]">
                <div className="flex gap-3">
                  <span className="num min-w-0 flex-1 truncate text-ink-2" title={item.path}>
                    {describe(item)}
                  </span>
                  {item.sizeBytes > 0 && <span className="num shrink-0 text-ink-2">{formatBytes(item.sizeBytes)}</span>}
                </div>
                <div className="text-ink-3">{error}</div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function describe(item: Leftover) {
  return item.label ?? (item.valueName ? `${item.path} → ${item.valueName}` : item.path);
}

const KIND_ICON: Record<Leftover["kind"], React.ReactNode> = {
  folder: <Folder />,
  file: <FileText />,
  registryKey: <ListTree />,
  registryValue: <KeyRound />,
  service: <Cog />,
  scheduledTask: <CalendarClock />,
  firewallRule: <Shield />,
};

const KIND_GROUP: Record<Leftover["kind"], number> = {
  folder: 0,
  file: 0,
  registryKey: 1,
  registryValue: 1,
  service: 2,
  scheduledTask: 2,
  firewallRule: 2,
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
    const groups = [
      { label: "Files and folders", ids: [] as number[] },
      { label: "Registry", ids: [] as number[] },
      { label: "Services, tasks, and firewall rules", ids: [] as number[] },
    ];
    leftovers.forEach((l, i) => groups[KIND_GROUP[l.kind]].ids.push(i));
    return groups.filter((g) => g.ids.length > 0);
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
                      <Checkbox label={describe(l)} checked={selected.has(i)} onChange={(on) => toggle(i, on)} className="mt-0.5" />
                      <span className="mt-0.5 text-ink-3 [&_svg]:size-4">{KIND_ICON[l.kind]}</span>
                      <div className="min-w-0">
                        <div className="num truncate text-[12px] text-ink" title={l.path}>
                          {l.label ?? l.path}
                          {!l.label && l.valueName && <span className="text-uv-300"> → {l.valueName}</span>}
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
