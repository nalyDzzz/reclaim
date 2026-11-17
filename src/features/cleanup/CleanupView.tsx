import { useCallback, useEffect, useState } from "react";
import { CircleCheck, FolderOpen, Info, Lock, RefreshCw, ShieldHalf, Sparkles, TriangleAlert } from "lucide-react";
import { api, errorMessage, type CleanReport, type CleanupTarget, type TargetReport } from "../../lib/api";
import { bytesParts, formatBytes, plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { Dialog } from "../../components/Dialog";
import { relaunchAsAdmin } from "../debloat/admin";
import { Badge, Button, Checkbox, cx, IconButton, PageHeader, Spinner } from "../../components/ui";

const GROUPS: { id: CleanupTarget["category"]; label: string; note?: string }[] = [
  {
    id: "large",
    label: "Bigger cleanups",
    note: "Each one frees a lot of space and gives something up, so they're chosen one at a time.",
  },
  { id: "system", label: "Windows" },
  { id: "browsers", label: "Browsers" },
  { id: "apps", label: "Apps" },
  { id: "developer", label: "Developer caches" },
];

/** The target being cleaned right now. */
interface Active {
  id: string;
  progress: number | null;
}

export function CleanupView() {
  const { addReclaimed, toast, system } = useApp();
  const [targets, setTargets] = useState<CleanupTarget[]>([]);
  const [reports, setReports] = useState<Record<string, TargetReport>>({});
  const [results, setResults] = useState<Record<string, CleanReport>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [analyzing, setAnalyzing] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [active, setActive] = useState<Active | null>(null);
  const [confirming, setConfirming] = useState(false);

  const analyze = useCallback(
    async (list: CleanupTarget[], fresh = false) => {
      setAnalyzing(true);
      try {
        setReports({});
        const found = await api.analyzeCleanup(
          list.map((t) => t.id),
          (r) => setReports((all) => ({ ...all, [r.id]: r })),
          fresh,
        );
        setReports(Object.fromEntries(found.map((r) => [r.id, r])));
      } catch (e) {
        toast("Couldn't check for junk files", { detail: errorMessage(e), tone: "danger" });
      } finally {
        setAnalyzing(false);
      }
    },
    [toast],
  );

  useEffect(() => {
    api.cleanupTargets().then((list) => {
      setTargets(list);
      setSelected(new Set(list.filter((t) => t.defaultSelected).map((t) => t.id)));
      analyze(list);
    });
  }, [analyze]);

  const analyzed = !analyzing && Object.keys(reports).length > 0;
  const isCleanable = (id: string) => {
    const r = reports[id];
    return !!r && !r.blocked && r.bytes > 0;
  };
  const visibleTargets = targets.filter((t) => !analyzed || reports[t.id]?.found);
  const chosen = visibleTargets.filter((t) => selected.has(t.id) && isCleanable(t.id));
  const chosenBytes = chosen.reduce((s, t) => s + reports[t.id].bytes, 0);
  const foundBytes = visibleTargets.reduce((s, t) => s + (reports[t.id]?.blocked ? 0 : (reports[t.id]?.bytes ?? 0)), 0);
  const activeTarget = active && targets.find((t) => t.id === active.id);
  // Windows' own cleanups scan for a minute after everything else is in.
  const waitingOnWindows =
    analyzing && targets.some((t) => !reports[t.id]) && targets.every((t) => reports[t.id] || t.category === "large");

  const clean = async () => {
    setConfirming(false);
    setCleaning(true);
    setResults({});
    try {
      const done = await api.runCleanup(
        chosen.map((t) => t.id),
        (step) => {
          if (step.report) {
            const report = step.report;
            setResults((all) => ({ ...all, [step.id]: report }));
            setActive(null);
          } else {
            setActive({ id: step.id, progress: step.progress });
          }
        },
      );
      const freed = done.reduce((s, r) => s + r.freedBytes, 0);
      const failed = done.reduce((s, r) => s + r.failedFiles, 0);
      const stopped = done.filter((r) => r.error);
      addReclaimed(freed);
      const details = [
        failed > 0 ? `${plural(failed, "file")} were in use and skipped.` : null,
        ...stopped.map((r) => `${targets.find((t) => t.id === r.id)?.name ?? r.id}: ${r.error}`),
      ].filter(Boolean);
      toast(stopped.length > 0 && freed === 0 ? "Cleanup didn't finish" : `Freed ${formatBytes(freed)}`, {
        tone: stopped.length > 0 ? "warn" : "good",
        detail: details.length > 0 ? details.join(" ") : undefined,
      });
      await analyze(targets);
    } catch (e) {
      toast("Cleanup stopped", { detail: errorMessage(e), tone: "danger" });
    } finally {
      setActive(null);
      setCleaning(false);
    }
  };

  const toggle = (id: string, on: boolean) =>
    setSelected((s) => {
      const next = new Set(s);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const { value, unit } = bytesParts(chosenBytes);
  const chosenTradeOffs = chosen.filter((t) => t.givesUp);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Cleanup"
        meta="Temporary files and caches that apps and Windows rebuild on their own, and bigger cleanups you choose yourself."
        actions={
          <IconButton label="Check again" onClick={() => analyze(targets, true)} disabled={analyzing || cleaning}>
            <RefreshCw className={cx("size-4", analyzing && "animate-spin")} />
          </IconButton>
        }
      />

      <div className="mx-7 mb-4 flex items-center gap-6 rounded-xl border border-line bg-white/[0.02] px-5 py-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="display text-[34px] leading-none text-ink tabular-nums">{value}</span>
            <span className="text-[15px] text-ink-2">{unit} selected</span>
          </div>
          <div className="mt-1.5 truncate text-[12.5px] text-ink-3">
            {cleaning && activeTarget
              ? `Cleaning ${activeTarget.name}${active?.progress != null ? `, ${Math.round(active.progress * 100)}%` : ""}…`
              : waitingOnWindows
                ? "Asking Windows how much its own cleanups would free. This can take a minute…"
                : analyzing
                  ? "Checking caches and temp folders…"
                : `${formatBytes(foundBytes)} found across ${plural(visibleTargets.length, "location")}`}
          </div>
        </div>
        <Button
          variant="primary"
          icon={<Sparkles className="size-3.5" />}
          disabled={chosen.length === 0 || analyzing}
          busy={cleaning}
          onClick={() => setConfirming(true)}
        >
          {cleaning ? "Cleaning" : `Clean ${plural(chosen.length, "item")}`}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-8">
        {GROUPS.map((g) => {
          const items = visibleTargets.filter((t) => t.category === g.id);
          if (items.length === 0) return null;
          const selectable = items.filter((t) => isCleanable(t.id));
          const all = selectable.length > 0 && selectable.every((t) => selected.has(t.id));
          const some = selectable.some((t) => selected.has(t.id));
          // Each bigger cleanup gives something up, so there's no "select all".
          const pickOneByOne = g.id === "large";
          const needsAdmin = !system?.elevated && items.some((t) => reports[t.id]?.blocked);
          return (
            <section key={g.id} className="mb-5">
              <header className="mb-1 flex items-start gap-3 px-3 py-1.5">
                {!pickOneByOne && (
                  <Checkbox
                    label={`Select all ${g.label}`}
                    checked={all}
                    indeterminate={!all && some}
                    disabled={selectable.length === 0}
                    onChange={(on) => selectable.forEach((t) => toggle(t.id, on))}
                  />
                )}
                <div className="min-w-0 flex-1">
                  <h2 className="text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">{g.label}</h2>
                  {g.note && <p className="mt-0.5 text-[12.5px] text-ink-3">{g.note}</p>}
                </div>
                {pickOneByOne && needsAdmin && (
                  <Button size="sm" icon={<ShieldHalf className="size-3.5 text-warn" />} onClick={() => relaunchAsAdmin(toast)}>
                    Restart as administrator
                  </Button>
                )}
              </header>
              <ul className="overflow-hidden rounded-lg border border-line">
                {items.map((t) => (
                  <TargetRow
                    key={t.id}
                    target={t}
                    report={reports[t.id]}
                    result={results[t.id]}
                    analyzing={analyzing}
                    active={active?.id === t.id ? active : null}
                    checked={selected.has(t.id)}
                    onToggle={(on) => toggle(t.id, on)}
                  />
                ))}
              </ul>
            </section>
          );
        })}
      </div>

      {confirming && (
        <Dialog
          title={`Clean ${plural(chosen.length, "item")}?`}
          onClose={() => setConfirming(false)}
          footer={
            <>
              <Button variant="ghost" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button variant="primary" onClick={clean} data-autofocus>
                Clean {formatBytes(chosenBytes)}
              </Button>
            </>
          }
        >
          <p className="mb-3">
            {chosenTradeOffs.length > 0
              ? "Nothing here can be brought back afterward. Files that are in use are skipped."
              : "These files are deleted permanently. Files that are in use are skipped."}
          </p>
          <ul className="flex flex-col divide-y divide-line rounded-lg border border-line">
            {chosen.map((t) => (
              <li key={t.id} className="px-3 py-2">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-ink">{t.name}</span>
                  <span className="num text-[12.5px] text-ink-2">{formatBytes(reports[t.id].bytes)}</span>
                </div>
                {t.givesUp && <TradeOff text={t.givesUp} />}
                {t.givesUp && reports[t.id].note && <Note text={reports[t.id].note!} />}
              </li>
            ))}
          </ul>
        </Dialog>
      )}
    </div>
  );
}

function TradeOff({ text }: { text: string }) {
  return (
    <div className="mt-1.5 flex items-start gap-1.5 text-[12px] text-ink-2">
      <TriangleAlert className="mt-px size-3.5 shrink-0 text-warn" />
      <span>{text}</span>
    </div>
  );
}

function Note({ text }: { text: string }) {
  return (
    <div className="mt-1.5 flex items-start gap-1.5 text-[12px] text-ink-2">
      <Info className="mt-px size-3.5 shrink-0 text-ink-3" />
      <span>{text}</span>
    </div>
  );
}

function TargetRow({
  target: t,
  report,
  result,
  analyzing,
  active,
  checked,
  onToggle,
}: {
  target: CleanupTarget;
  report: TargetReport | undefined;
  result: CleanReport | undefined;
  analyzing: boolean;
  active: Active | null;
  checked: boolean;
  onToggle: (on: boolean) => void;
}) {
  const blocked = report?.blocked;
  // Admin-only locations may be unreadable, so zero there means "unknown".
  const empty = report && report.bytes === 0 && !blocked;
  const disabled = !report || blocked || empty;
  const showResult = result && (result.freedBytes > 0 || result.failedFiles > 0 || result.error);
  return (
    <li
      onClick={() => !disabled && onToggle(!checked)}
      className={cx(
        "grid cursor-default grid-cols-[16px_minmax(0,1fr)_auto] items-start gap-3.5 border-b border-line/60 px-3 py-3 last:border-b-0",
        !disabled && "hover:bg-white/3",
      )}
    >
      <Checkbox label={t.name} checked={checked && !disabled} disabled={disabled} onChange={onToggle} className="mt-0.5" />
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <span className={cx("text-[13.5px]", disabled ? "text-ink-2" : "text-ink")}>{t.name}</span>
          {blocked && (
            <Badge tone="warn" icon={<Lock />}>
              Needs administrator
            </Badge>
          )}
          {report?.running && !blocked && (
            <Badge tone="warn" icon={<TriangleAlert />}>
              {t.category === "system" ? "In use by Windows, some files will be skipped" : `${t.name} is open, close it for a full clean`}
            </Badge>
          )}
        </div>
        <div className="mt-0.5 text-[12.5px] text-ink-3">{t.description}</div>
        {t.givesUp && <TradeOff text={t.givesUp} />}
        {report?.note && <Note text={report.note} />}
        {report?.reveal && (
          <Button
            size="sm"
            variant="ghost"
            icon={<FolderOpen className="size-3.5" />}
            className="mt-1 -ml-2.5"
            onClick={(e) => {
              e.stopPropagation();
              api.openPath(report.reveal!).catch(() => api.revealPath(report.reveal!));
            }}
          >
            Open folder
          </Button>
        )}
        {showResult && (
          <div className="mt-1.5 flex items-start gap-2 text-[12px]">
            {result.error ? (
              <TriangleAlert className="mt-px size-3.5 shrink-0 text-warn" />
            ) : (
              <CircleCheck className="mt-px size-3.5 shrink-0 text-good" />
            )}
            <span className="text-ink-2">
              {result.freedBytes > 0 && `Freed ${formatBytes(result.freedBytes)}`}
              {result.freedBytes > 0 && (result.failedFiles > 0 || result.error) && " · "}
              {result.failedFiles > 0 && <span className="text-ink-3">{plural(result.failedFiles, "file")} in use, skipped</span>}
              {result.error}
            </span>
          </div>
        )}
      </div>
      <div className="flex min-w-[88px] flex-col items-end pt-0.5">
        {active ? (
          <>
            <Spinner className="size-3.5 text-uv-300" />
            <span className="num mt-1 text-[11px] text-ink-3">
              {active.progress != null ? `${Math.round(active.progress * 100)}%` : "Cleaning"}
            </span>
          </>
        ) : !report && analyzing ? (
          <Spinner className="size-3.5" />
        ) : empty ? (
          <span className="text-[12px] text-ink-3">Nothing to clean</span>
        ) : report && blocked && report.bytes === 0 ? (
          <span className="text-[12px] text-ink-3">—</span>
        ) : report ? (
          <>
            <span className={cx("num text-[13px]", blocked ? "text-ink-3" : "text-ink")}>{formatBytes(report.bytes)}</span>
            {report.files > 0 && <span className="num text-[11px] text-ink-3">{plural(report.files, "file")}</span>}
          </>
        ) : null}
      </div>
    </li>
  );
}
