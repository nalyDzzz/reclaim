import { useCallback, useEffect, useMemo, useState } from "react";
import { History, Lock, PackageX, RotateCcw, ShieldHalf, Store } from "lucide-react";
import { api, errorMessage, type AppxApp, type Tweak } from "../../lib/api";
import { plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { useDebloat } from "../../state/debloat";
import { Dialog } from "../../components/Dialog";
import { Badge, Button, cx, EmptyState, PageHeader, SearchField, Spinner, Switch, Tabs } from "../../components/ui";

type Section = "apps" | "settings";

export function DebloatView() {
  const [section, setSection] = useState<Section>("apps");
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Debloat"
        meta="Remove Store apps you don't use and turn off ads, suggestions, and tracking."
        actions={
          <Tabs
            value={section}
            onChange={setSection}
            options={[
              { value: "apps", label: "Store apps" },
              { value: "settings", label: "Windows settings" },
            ]}
          />
        }
      />
      <DriftBanner />
      <ExplorerRestartBanner />
      {section === "apps" ? <PreinstalledApps /> : <WindowsSettings />}
    </div>
  );
}

/** Changes Windows undid since the user made them, with a way to redo them. */
function DriftBanner() {
  const { drift, driftCount, redo, redoing, keepAsIs } = useDebloat();
  if (!drift || driftCount === 0) return null;
  const blocked = drift.tweaks.filter((t) => t.blocked).length;
  return (
    <div className="mx-7 mb-4 flex items-start gap-3 rounded-lg border border-warn/35 bg-warn/8 px-4 py-3">
      <History className="mt-0.5 size-4 shrink-0 text-warn" />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-ink">Some of your changes came undone</div>
        <div className="mt-0.5 text-[12.5px] text-ink-2">
          Windows updates often put back settings and apps. Redo your changes, or keep things as they are now.
        </div>
        <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-[12.5px]">
          {drift.tweaks.length > 0 && (
            <>
              <dt className="text-ink-3">Settings reset</dt>
              <dd className="text-ink">
                {drift.tweaks.map((t) => t.name).join(", ")}
                {blocked > 0 && (
                  <span className="ml-2 inline-flex align-middle">
                    <Badge tone="warn" icon={<Lock />}>
                      {blocked === drift.tweaks.length ? "Needs administrator" : `${blocked} ${blocked === 1 ? "needs" : "need"} administrator`}
                    </Badge>
                  </span>
                )}
              </dd>
            </>
          )}
          {drift.apps.length > 0 && (
            <>
              <dt className="text-ink-3">Apps back</dt>
              <dd className="text-ink">{drift.apps.map((a) => a.displayName).join(", ")}</dd>
            </>
          )}
        </dl>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button size="sm" variant="ghost" disabled={redoing} onClick={keepAsIs}>
          Keep as is
        </Button>
        <Button size="sm" variant="primary" busy={redoing} onClick={redo}>
          Redo {plural(driftCount, "change")}
        </Button>
      </div>
    </div>
  );
}

function ExplorerRestartBanner() {
  const { toast } = useApp();
  const { needsExplorerRestart, setNeedsExplorerRestart } = useDebloat();
  const [restarting, setRestarting] = useState(false);
  if (!needsExplorerRestart) return null;
  return (
    <div className="mx-7 mb-4 flex items-center gap-3 rounded-lg border border-uv-500/40 bg-uv-600/10 px-4 py-3">
      <RotateCcw className="size-4 shrink-0 text-uv-300" />
      <span className="flex-1 text-[13px] text-ink">Some changes apply after File Explorer restarts. Open Explorer windows will close.</span>
      <Button
        size="sm"
        busy={restarting}
        onClick={async () => {
          setRestarting(true);
          await api.restartExplorer().catch((e) => toast("Couldn't restart File Explorer", { detail: errorMessage(e), tone: "danger" }));
          setRestarting(false);
          setNeedsExplorerRestart(false);
        }}
      >
        Restart File Explorer
      </Button>
    </div>
  );
}

const GUID = /^[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$/i;

const logoCache = new Map<string, Promise<string | null>>();

function AppLogo({ location, name }: { location: string | null; name: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!location) return;
    let alive = true;
    let pending = logoCache.get(location);
    if (!pending) {
      pending = api.appxLogo(location).catch(() => null);
      logoCache.set(location, pending);
    }
    pending.then((url) => alive && setSrc(url));
    return () => {
      alive = false;
    };
  }, [location]);
  return (
    <span className="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-md bg-white/5 ring-1 ring-line">
      {src ? (
        <img src={src} alt="" className="size-6 object-contain" draggable={false} />
      ) : (
        <span className="text-[13px] font-semibold text-ink-3">{name.charAt(0)}</span>
      )}
    </span>
  );
}

function relaunchAsAdmin(toast: ReturnType<typeof useApp>["toast"]) {
  api.relaunchAsAdmin().catch((e) => toast("Reclaim is still running without admin rights", { detail: errorMessage(e), tone: "warn" }));
}

function PreinstalledApps() {
  const { toast, system } = useApp();
  const { revision, settleApp } = useDebloat();
  const elevated = system?.elevated ?? false;
  const [apps, setApps] = useState<AppxApp[] | null>(null);
  const [query, setQuery] = useState("");
  const [removing, setRemoving] = useState<Set<string>>(new Set());
  const [bulk, setBulk] = useState<AppxApp[] | null>(null);

  const load = useCallback(() => {
    api
      .listAppx()
      .then(setApps)
      .catch((e) => {
        toast("Couldn't list Store apps", { detail: errorMessage(e), tone: "danger" });
        setApps([]);
      });
  }, [toast]);

  useEffect(load, [load, revision]);

  const remove = async (list: AppxApp[]) => {
    setBulk(null);
    setRemoving((s) => new Set([...s, ...list.map((a) => a.packageFullName)]));
    let removed = 0;
    for (const app of list) {
      try {
        await api.removeAppx(app.name, app.installed ? app.packageFullName : null);
        removed++;
        settleApp(app.name);
        // Without administrator rights Windows' copy stays, so the app moves to that section.
        const copyStays = !elevated && app.installed && app.provisioned && !app.keepsCopy;
        setApps(
          (all) =>
            all?.flatMap((a) => (a.packageFullName !== app.packageFullName ? [a] : copyStays ? [{ ...a, installed: false }] : [])) ?? null,
        );
      } catch (e) {
        toast(`Couldn't remove ${app.displayName}`, { detail: errorMessage(e), tone: "danger" });
      } finally {
        setRemoving((s) => {
          const next = new Set(s);
          next.delete(app.packageFullName);
          return next;
        });
      }
    }
    if (removed > 0) toast(`Removed ${plural(removed, "app")}`, { tone: "good" });
  };

  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (a: AppxApp) => !q || a.displayName.toLowerCase().includes(q) || a.name.toLowerCase().includes(q) || a.publisher.toLowerCase().includes(q);
  }, [query]);
  const installed = useMemo(() => (apps ?? []).filter((a) => a.installed), [apps]);
  const copies = useMemo(() => (apps ?? []).filter((a) => !a.installed), [apps]);
  const visibleInstalled = installed.filter(matches);
  const visibleCopies = copies.filter(matches);
  const recommended = installed.filter((a) => a.recommended);

  if (!apps) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 text-[13px] text-ink-2">
        <Spinner className="size-5" />
        Reading installed Store apps…
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center gap-3 px-7 pb-3">
        <SearchField value={query} onChange={setQuery} placeholder="Search Store apps" className="w-72" />
        <span className="flex-1 text-[12.5px] text-ink-3">{plural(installed.length, "removable app")}</span>
        {recommended.length > 0 && (
          <Button variant="primary" icon={<PackageX className="size-3.5" />} onClick={() => setBulk(recommended)}>
            Remove {plural(recommended.length, "common bloat app")}
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto border-t border-line">
        {visibleInstalled.length === 0 && visibleCopies.length === 0 ? (
          <EmptyState icon={<Store />} title={query ? `No apps match “${query}”` : "No removable Store apps"} />
        ) : (
          visibleInstalled.map((a) => <AppRow key={a.packageFullName} app={a} busy={removing.has(a.packageFullName)} onRemove={() => remove([a])} />)
        )}

        {visibleCopies.length > 0 && (
          <section aria-label="Copies Windows keeps">
            <div className="flex items-start gap-4 border-b border-line/60 bg-white/2 px-7 pt-5 pb-3">
              <div className="min-w-0 flex-1">
                <h2 className="text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">Removed for you, but Windows keeps a copy</h2>
                <p className="mt-1 max-w-2xl text-[12.5px] text-ink-2">
                  Windows sets these up for every new account and can put them back during a feature update. Removing the copy keeps them
                  gone.
                </p>
              </div>
              {elevated ? (
                <Button size="sm" icon={<PackageX className="size-3.5" />} onClick={() => setBulk(copies)}>
                  Remove {plural(copies.length, "copy", "copies")}
                </Button>
              ) : (
                <Button size="sm" icon={<ShieldHalf className="size-3.5 text-warn" />} onClick={() => relaunchAsAdmin(toast)}>
                  Restart as administrator
                </Button>
              )}
            </div>
            {visibleCopies.map((a) => (
              <AppRow
                key={a.packageFullName}
                app={a}
                busy={removing.has(a.packageFullName)}
                blocked={!elevated}
                onRemove={() => remove([a])}
              />
            ))}
          </section>
        )}
      </div>

      {bulk && (
        <Dialog
          title={`Remove ${plural(bulk.length, "app")}?`}
          onClose={() => setBulk(null)}
          footer={
            <>
              <Button variant="ghost" onClick={() => setBulk(null)}>
                Cancel
              </Button>
              <Button variant="danger" onClick={() => remove(bulk)} data-autofocus>
                Remove {plural(bulk.length, "app")}
              </Button>
            </>
          }
        >
          <p className="mb-3">
            {bulk.every((a) => !a.installed)
              ? "Windows' copies of these are removed, so they won't come back for new accounts or after updates."
              : elevated
                ? "These are removed for your account, along with the copies Windows keeps, so they won't come back for new accounts or after updates."
                : "These are removed for your account. Windows keeps a copy of most and can put them back after a big update. To remove those too, restart Reclaim as administrator."}{" "}
            You can reinstall any of them from the Microsoft Store.
          </p>
          <ul className="flex flex-wrap gap-1.5">
            {bulk.map((a) => (
              <li key={a.packageFullName}>
                <Badge>{a.displayName}</Badge>
              </li>
            ))}
          </ul>
        </Dialog>
      )}
    </>
  );
}

function AppRow({ app: a, busy, blocked, onRemove }: { app: AppxApp; busy: boolean; blocked?: boolean; onRemove: () => void }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 border-b border-line/60 px-7 py-2.5 hover:bg-white/3">
      <div className="flex min-w-0 items-center gap-3">
        <AppLogo location={a.installLocation} name={a.displayName} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-[13.5px] text-ink">{a.displayName}</span>
            {a.recommended && <Badge tone="uv">Commonly removed</Badge>}
          </div>
          <div className="truncate text-[12px] text-ink-3">
            {a.note ?? (GUID.test(a.publisher) || !a.publisher ? "Microsoft Store app" : a.publisher)}
            <span className="num ml-2 text-ink-3/70">{a.name}</span>
          </div>
        </div>
      </div>
      <Button size="sm" busy={busy} disabled={blocked} onClick={onRemove} title={blocked ? "Needs administrator rights" : undefined}>
        {a.installed ? "Remove" : "Remove copy"}
      </Button>
    </div>
  );
}

const TWEAK_GROUPS: { id: Tweak["category"]; label: string }[] = [
  { id: "privacy", label: "Privacy" },
  { id: "start", label: "Start, search, and lock screen" },
  { id: "explorer", label: "File Explorer and taskbar" },
];

function WindowsSettings() {
  const { toast } = useApp();
  const { revision, settleTweak, setNeedsExplorerRestart } = useDebloat();
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
      {TWEAK_GROUPS.map((g) => (
        <section key={g.id} className="mb-5">
          <h2 className="mb-1 px-3 py-1.5 text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">{g.label}</h2>
          <ul className="overflow-hidden rounded-lg border border-line">
            {tweaks
              .filter((t) => t.category === g.id)
              .map((t) => (
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
      ))}
      <p className="px-3 text-[12px] text-ink-3">
        Each switch only changes its own setting. Turning it off restores the Windows default. Reclaim checks on each start whether Windows
        has undone any of them.
      </p>
    </div>
  );
}
