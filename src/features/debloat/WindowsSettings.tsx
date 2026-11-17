import { useEffect, useState } from "react";
import { Lock } from "lucide-react";
import { api, errorMessage, type Tweak } from "../../lib/api";
import { useApp } from "../../state/app";
import { useDebloat } from "../../state/debloat";
import { Badge, cx, Spinner, Switch } from "../../components/ui";

const TWEAK_GROUPS: { id: Tweak["category"]; label: string }[] = [
  { id: "privacy", label: "Privacy" },
  { id: "start", label: "Ads, suggestions, and search" },
  { id: "taskbar", label: "Taskbar" },
  { id: "explorer", label: "File Explorer" },
  { id: "gaming", label: "Gaming" },
  { id: "system", label: "System" },
];

export function WindowsSettings() {
  const { toast } = useApp();
  const { revision, settleTweak, setNeedsExplorerRestart, setNeedsSignIn } = useDebloat();
  const [tweaks, setTweaks] = useState<Tweak[] | null>(null);
  const [pending, setPending] = useState<Set<string>>(new Set());

  useEffect(() => {
    api
      .listTweaks()
      .then(setTweaks)
      .catch(() => setTweaks([]));
  }, [revision]);

  const toggle = async (t: Tweak, on: boolean) => {
    setPending((s) => new Set(s).add(t.id));
    try {
      await api.setTweak(t.id, on);
      setTweaks((list) => list?.map((x) => (x.id === t.id ? { ...x, applied: on } : x)) ?? null);
      settleTweak(t.id);
      if (t.restartExplorer) setNeedsExplorerRestart(true);
      if (t.signOut) setNeedsSignIn(true);
    } catch (e) {
      toast(`Couldn't change “${t.name}”`, { detail: errorMessage(e), tone: "danger" });
    } finally {
      setPending((s) => {
        const next = new Set(s);
        next.delete(t.id);
        return next;
      });
    }
  };

  if (!tweaks) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-8">
      {TWEAK_GROUPS.map((g) => {
        const group = tweaks.filter((t) => t.category === g.id);
        // Older versions of Windows don't have every setting.
        if (group.length === 0) return null;
        return (
          <section key={g.id} className="mb-5">
            <h2 className="mb-1 px-3 py-1.5 text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">{g.label}</h2>
            <ul className="overflow-hidden rounded-lg border border-line">
              {group.map((t) => (
                <li
                  key={t.id}
                  className={cx("flex items-center gap-4 border-b border-line/60 px-4 py-3 last:border-b-0", t.blocked && "opacity-70")}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-[13.5px] text-ink">{t.name}</span>
                      {t.blocked && (
                        <Badge tone="warn" icon={<Lock />}>
                          Needs administrator
                        </Badge>
                      )}
                    </div>
                    <div className="mt-0.5 text-[12.5px] text-ink-3">{t.description}</div>
                  </div>
                  {pending.has(t.id) && <Spinner className="size-3.5" />}
                  <Switch label={t.name} checked={t.applied} disabled={t.blocked || pending.has(t.id)} onChange={(on) => toggle(t, on)} />
                </li>
              ))}
            </ul>
          </section>
        );
      })}
      <p className="px-3 text-[12px] text-ink-3">
        Each switch only changes its own setting. Turning it off restores the Windows default. Reclaim checks on each start whether Windows
        has turned ads, suggestions, or tracking back on.
      </p>
    </div>
  );
}
