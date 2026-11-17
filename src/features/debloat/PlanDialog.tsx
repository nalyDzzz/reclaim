import { useState } from "react";
import { Check, Lock, PackageX, TriangleAlert } from "lucide-react";
import { api, errorMessage } from "../../lib/api";
import { plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { useDebloat } from "../../state/debloat";
import { Dialog } from "../../components/Dialog";
import { Badge, Button, Checkbox, Meter, Spinner } from "../../components/ui";
import { relaunchAsAdmin } from "./admin";
import { planSize, type Plan } from "./plan";

type Phase =
  | { kind: "review" }
  | { kind: "restoring" }
  | { kind: "noRestorePoint"; message: string }
  | { kind: "applying"; done: number; current: string };

/** Shows every change in a plan, then makes them, after a restore point if wanted. */
export function PlanDialog({ plan, onClose }: { plan: Plan; onClose: () => void }) {
  const { system, toast } = useApp();
  const { afterChanges } = useDebloat();
  const elevated = system?.elevated ?? false;
  const [restorePoint, setRestorePoint] = useState(elevated);
  const [phase, setPhase] = useState<Phase>({ kind: "review" });
  const total = planSize(plan);
  const skipped = plan.blockedTweaks.length + plan.blockedCopies.length;

  const apply = async () => {
    const failed: string[] = [];
    let done = 0;
    let explorer = false;
    let signIn = false;
    for (const t of plan.tweaks) {
      setPhase({ kind: "applying", done, current: t.name });
      try {
        await api.setTweak(t.id, true);
        explorer ||= t.restartExplorer;
        signIn ||= t.signOut;
      } catch (e) {
        failed.push(`${t.name}: ${errorMessage(e)}`);
      }
      done++;
    }
    for (const a of plan.apps) {
      setPhase({
        kind: "applying",
        done,
        current: a.installed ? `Removing ${a.displayName}` : `Removing Windows' copy of ${a.displayName}`,
      });
      try {
        await api.removeAppx(a.name, a.installed ? a.packageFullName : null);
      } catch (e) {
        failed.push(`${a.displayName}: ${errorMessage(e)}`);
      }
      done++;
    }
    afterChanges({ explorer, signIn });
    onClose();
    const made = total - failed.length;
    if (failed.length) {
      toast(`Made ${plural(made, "change")}, ${failed.length} didn't work`, { detail: failed.join("\n"), tone: "danger" });
    } else {
      toast(`Made ${plural(made, "change")}`, { tone: "good" });
    }
  };

  const start = async () => {
    if (!restorePoint) return apply();
    setPhase({ kind: "restoring" });
    try {
      const point = await api.createRestorePoint();
      if (point.created) return apply();
      setPhase({ kind: "noRestorePoint", message: point.note ?? "Windows didn't make a restore point." });
    } catch (e) {
      setPhase({ kind: "noRestorePoint", message: errorMessage(e) });
    }
  };

  if (phase.kind === "restoring" || phase.kind === "applying") {
    return (
      <Dialog title={plan.title}>
        {phase.kind === "restoring" ? (
          <div className="flex items-center gap-3 py-2">
            <Spinner className="size-4" />
            <span>Making a restore point. Windows can take a minute with this.</span>
          </div>
        ) : (
          <div className="flex flex-col gap-2.5 py-1">
            <div className="flex items-baseline justify-between gap-3">
              <span className="truncate text-ink">{phase.current}</span>
              <span className="num shrink-0 text-[12px] text-ink-3">
                {phase.done + 1} of {total}
              </span>
            </div>
            <Meter fraction={phase.done / total} />
          </div>
        )}
      </Dialog>
    );
  }

  if (phase.kind === "noRestorePoint") {
    return (
      <Dialog
        title="No restore point was made"
        icon={<TriangleAlert className="text-warn" />}
        onClose={onClose}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" onClick={apply} data-autofocus>
              Make {plural(total, "change")} anyway
            </Button>
          </>
        }
      >
        <p>{phase.message}</p>
        <p className="mt-2">Every change can still be undone from the Store apps and Windows settings tabs.</p>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={plan.title}
      onClose={onClose}
      width={560}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {total === 0 ? "Close" : "Cancel"}
          </Button>
          {total > 0 && (
            <Button variant="primary" onClick={start} data-autofocus>
              Make {plural(total, "change")}
            </Button>
          )}
        </>
      }
    >
      <p>{plan.intro}</p>

      {total === 0 && skipped === 0 && (
        <p className="mt-4 flex items-center gap-2 text-ink">
          <Check className="size-4 text-good" /> Everything here is already done on this PC.
        </p>
      )}

      {plan.tweaks.length > 0 && (
        <section className="mt-4">
          <h3 className="mb-1.5 text-[12px] font-semibold tracking-[0.08em] text-ink-3 uppercase">
            Turn on {plural(plan.tweaks.length, "setting")}
          </h3>
          <ul className="flex flex-col gap-1">
            {plan.tweaks.map((t) => (
              <li key={t.id} className="flex items-center gap-2 text-ink">
                <Check className="size-3.5 shrink-0 text-uv-300" />
                {t.name}
              </li>
            ))}
          </ul>
        </section>
      )}

      {plan.apps.length > 0 && (
        <section className="mt-4">
          <h3 className="mb-1.5 text-[12px] font-semibold tracking-[0.08em] text-ink-3 uppercase">
            Remove {plural(plan.apps.length, "app")}
          </h3>
          <ul className="flex flex-wrap gap-1.5">
            {plan.apps.map((a) => (
              <li key={a.packageFullName}>
                <Badge icon={a.installed ? <PackageX /> : undefined}>
                  {a.installed ? a.displayName : `${a.displayName} (Windows' copy)`}
                </Badge>
              </li>
            ))}
          </ul>
          <p className="mt-2 text-[12px] text-ink-3">
            {elevated
              ? "Windows' copies go too, so they won't come back after updates."
              : "Removed for your account. Windows keeps a copy of most and can put them back after a big update."}{" "}
            Any of them can be reinstalled from the Microsoft Store.
          </p>
        </section>
      )}

      {skipped > 0 && (
        <section className="mt-4 rounded-lg border border-line px-3.5 py-3">
          <div className="flex items-start gap-2.5">
            <Lock className="mt-0.5 size-3.5 shrink-0 text-warn" />
            <div className="min-w-0 flex-1">
              <div className="text-ink">Skipped without administrator rights</div>
              <div className="mt-0.5 text-[12.5px]">
                {[
                  ...plan.blockedTweaks.map((t) => t.name),
                  ...(plan.blockedCopies.length ? [`Windows' copies of ${plural(plan.blockedCopies.length, "app")}`] : []),
                ].join(", ")}
              </div>
            </div>
            <Button size="sm" onClick={() => relaunchAsAdmin(toast)}>
              Restart as administrator
            </Button>
          </div>
        </section>
      )}

      {plan.note && <p className="mt-4 text-[12.5px] text-ink-3">{plan.note}</p>}

      {total > 0 && (
        <label className="mt-5 flex items-start gap-2.5">
          <Checkbox
            label="Make a restore point first"
            checked={restorePoint}
            disabled={!elevated}
            onChange={setRestorePoint}
            className="mt-0.5"
          />
          <span className={elevated ? undefined : "opacity-60"}>
            <span className="text-ink">Make a restore point first</span>
            <span className="block text-[12px] text-ink-3">
              {elevated
                ? "Lets you roll everything back from Windows' System Restore. Takes up to a minute."
                : "Needs administrator rights."}
            </span>
          </span>
        </label>
      )}
    </Dialog>
  );
}
