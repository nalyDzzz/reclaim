import { useCallback, useEffect, useState } from "react";
import { CircleCheck, Lock, RefreshCw, Sparkles, TriangleAlert } from "lucide-react";
import { api, errorMessage, type CleanReport, type CleanupTarget, type TargetReport } from "../../lib/api";
import { bytesParts, formatBytes, plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { Dialog } from "../../components/Dialog";
import { Badge, Button, Checkbox, cx, IconButton, PageHeader, Spinner } from "../../components/ui";

const GROUPS: { id: CleanupTarget["category"]; label: string }[] = [
  { id: "system", label: "Windows" },
  { id: "browsers", label: "Browsers" },
  { id: "apps", label: "Apps" },
  { id: "developer", label: "Developer caches" },
];

export function CleanupView() {
  const { addReclaimed, toast } = useApp();
  const [targets, setTargets] = useState<CleanupTarget[]>([]);
  const [reports, setReports] = useState<Record<string, TargetReport>>({});
  const [results, setResults] = useState<Record<string, CleanReport>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [analyzing, setAnalyzing] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const analyze = useCallback(
    async (list: CleanupTarget[]) => {
      setAnalyzing(true);
      try {
        setReports({});
        const found = await api.analyzeCleanup(
          list.map((t) => t.id),
          (r) => setReports((all) => ({ ...all, [r.id]: r })),
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

  const clean = async () => {
    setConfirming(false);
    setCleaning(true);
    try {
      const done = await api.runCleanup(chosen.map((t) => t.id));
      setResults(Object.fromEntries(done.map((r) => [r.id, r])));
      const freed = done.reduce((s, r) => s + r.freedBytes, 0);
      const failed = done.reduce((s, r) => s + r.failedFiles, 0);
      addReclaimed(freed);
      toast(`Freed ${formatBytes(freed)}`, {
        tone: "good",
        detail: failed > 0 ? `${plural(failed, "file")} were in use and skipped.` : undefined,
      });
      await analyze(targets);
    } catch (e) {
      toast("Cleanup stopped", { detail: errorMessage(e), tone: "danger" });
    } finally {
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

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Cleanup"
        meta="Temporary files and caches that apps and Windows rebuild on their own."
        actions={
          <IconButton label="Check again" onClick={() => analyze(targets)} disabled={analyzing || cleaning}>
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
          <div className="mt-1.5 text-[12.5px] text-ink-3">
            {analyzing ? "Checking caches and temp folders…" : `${formatBytes(foundBytes)} found across ${plural(visibleTargets.length, "location")}`}
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
          return (
            <section key={g.id} className="mb-5">
              <header className="mb-1 flex items-center gap-3 px-3 py-1.5">
                <Checkbox
                  label={`Select all ${g.label}`}
                  checked={all}
                  indeterminate={!all && some}
                  disabled={selectable.length === 0}
                  onChange={(on) => selectable.forEach((t) => toggle(t.id, on))}
                />
                <h2 className="text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">{g.label}</h2>
              </header>
              <ul className="overflow-hidden rounded-lg border border-line">
                {items.map((t) => (
                  <TargetRow
                    key={t.id}
                    target={t}
                    report={reports[t.id]}
                    result={results[t.id]}
                    analyzing={analyzing}
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
          <p className="mb-3">These files are deleted permanently. Files that are in use are skipped.</p>
          <ul className="flex flex-col divide-y divide-line rounded-lg border border-line">
            {chosen.map((t) => (
              <li key={t.id} className="flex items-center justify-between gap-3 px-3 py-2">
                <span className="text-ink">{t.name}</span>
                <span className="num text-[12.5px] text-ink-2">{formatBytes(reports[t.id].bytes)}</span>
              </li>
            ))}
          </ul>
        </Dialog>
      )}
    </div>
  );
}

function TargetRow({
  target: t,
  report,
  result,
  analyzing,
  checked,
  onToggle,
}: {
  target: CleanupTarget;
  report: TargetReport | undefined;
  result: CleanReport | undefined;
  analyzing: boolean;
  checked: boolean;
  onToggle: (on: boolean) => void;
}) {
  const blocked = report?.blocked;
  // Admin-only locations may be unreadable, so zero there means "unknown".
  const empty = report && report.bytes === 0 && !blocked;
  const disabled = !report || blocked || empty;
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
        {result && (result.freedBytes > 0 || result.failedFiles > 0) && (
          <div className="mt-1.5 flex items-center gap-2 text-[12px]">
            <CircleCheck className="size-3.5 text-good" />
            <span className="text-ink-2">Freed {formatBytes(result.freedBytes)}</span>
            {result.failedFiles > 0 && <span className="text-ink-3">· {plural(result.failedFiles, "file")} in use, skipped</span>}
          </div>
        )}
      </div>
      <div className="flex min-w-[88px] flex-col items-end pt-0.5">
        {!report && analyzing ? (
          <Spinner className="size-3.5" />
        ) : empty ? (
          <span className="text-[12px] text-ink-3">Nothing to clean</span>
        ) : report && blocked && report.bytes === 0 ? (
          <span className="text-[12px] text-ink-3">—</span>
        ) : report ? (
          <>
            <span className={cx("num text-[13px]", blocked ? "text-ink-3" : "text-ink")}>{formatBytes(report.bytes)}</span>
            <span className="num text-[11px] text-ink-3">{plural(report.files, "file")}</span>
          </>
        ) : null}
      </div>
    </li>
  );
}
