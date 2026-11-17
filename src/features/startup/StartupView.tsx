import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
import { FolderOpen, Lock, RefreshCw, Rocket, ShieldHalf, TriangleAlert } from "lucide-react";
import { api, errorMessage, type StartupItem } from "../../lib/api";
import { plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { Badge, Button, cx, EmptyState, IconButton, PageHeader, SearchField, Spinner, Switch, Tabs } from "../../components/ui";

type Group = "apps" | "tasks" | "services";
type Filter = "all" | Group;

const GROUPS: { id: Group; label: string; about: string }[] = [
  { id: "apps", label: "Apps", about: "What Task Manager lists under Startup apps." },
  { id: "tasks", label: "Scheduled tasks", about: "Tasks that run when you sign in or Windows starts, often an app's updater." },
  {
    id: "services",
    label: "Services",
    about: "Background services from apps you installed. Turning one off sets it to start manually, so its app can still start it when needed.",
  },
];

function groupOf(item: StartupItem): Group {
  if (item.source === "task") return "tasks";
  if (item.source === "service") return "services";
  return "apps";
}

function whereFrom(item: StartupItem): string {
  switch (item.source) {
    case "registry":
      return item.allUsers ? "Registry, all users" : "Registry";
    case "folder":
      return item.allUsers ? "Startup folder, all users" : "Startup folder";
    case "task":
      return "Scheduled task";
    case "service":
      return item.delayed ? "Service, delayed start" : "Service";
  }
}

const iconCache = new Map<string, Promise<string | null>>();

function FileIcon({ path, name }: { path: string | null; name: string }) {
  const [src, setSrc] = useState<string | null | undefined>(path ? undefined : null);
  useEffect(() => {
    if (!path) return;
    let alive = true;
    let pending = iconCache.get(path);
    if (!pending) {
      pending = api.fileIcon(path).catch(() => null);
      iconCache.set(path, pending);
    }
    pending.then((url) => alive && setSrc(url));
    return () => {
      alive = false;
    };
  }, [path]);
  if (src) return <img src={src} alt="" width={28} height={28} className="shrink-0 object-contain" draggable={false} />;
  const letter = name.replace(/[^A-Za-z0-9]/g, "").charAt(0).toUpperCase() || "?";
  return (
    <span
      aria-hidden="true"
      className={cx(
        "flex size-7 shrink-0 items-center justify-center rounded-md bg-white/6 text-[12.5px] font-semibold text-ink-2 ring-1 ring-line",
        src === undefined && "opacity-0",
      )}
    >
      {letter}
    </span>
  );
}

export function StartupView() {
  const { toast, system } = useApp();
  const [items, setItems] = useState<StartupItem[] | null>(null);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [filter, setFilter] = useState<Filter>("all");
  const [pending, setPending] = useState<Set<string>>(new Set());

  const load = useCallback(() => {
    api
      .listStartup()
      .then(setItems)
      .catch((e) => {
        toast("Couldn't read what starts with Windows", { detail: errorMessage(e), tone: "danger" });
        setItems([]);
      });
  }, [toast]);

  useEffect(load, [load]);

  const toggle = async (item: StartupItem, on: boolean) => {
    setPending((s) => new Set(s).add(item.id));
    try {
      await api.setStartupItem(item.id, on);
      setItems((list) => list?.map((x) => (x.id === item.id ? { ...x, enabled: on } : x)) ?? null);
    } catch (e) {
      toast(`Couldn't change “${item.name}”`, { detail: errorMessage(e), tone: "danger" });
    } finally {
      setPending((s) => {
        const next = new Set(s);
        next.delete(item.id);
        return next;
      });
    }
  };

  const matching = useMemo(() => {
    const q = deferredQuery.trim().toLowerCase();
    return (items ?? []).filter(
      (i) =>
        !q ||
        i.name.toLowerCase().includes(q) ||
        i.entry.toLowerCase().includes(q) ||
        (i.publisher ?? "").toLowerCase().includes(q) ||
        i.command.toLowerCase().includes(q),
    );
  }, [items, deferredQuery]);

  const counts = useMemo(() => {
    const c = { apps: 0, tasks: 0, services: 0 };
    for (const i of items ?? []) c[groupOf(i)]++;
    return c;
  }, [items]);
  const enabled = (items ?? []).filter((i) => i.enabled).length;
  const blocked = (items ?? []).some((i) => i.blocked);
  const shownGroups = GROUPS.filter((g) => filter === "all" || filter === g.id);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Startup"
        meta={
          items ? (
            <>
              <span className="num text-ink">{enabled}</span> of {plural(items.length, "item")} start with Windows. Changes apply the next time
              you sign in.
            </>
          ) : (
            "Checking what starts with Windows…"
          )
        }
        actions={
          <IconButton label="Refresh list" onClick={load}>
            <RefreshCw className="size-4" />
          </IconButton>
        }
      />

      <div className="flex items-center gap-3 px-7 pb-3">
        <SearchField value={query} onChange={setQuery} placeholder="Search names, publishers, or commands" className="w-80" />
        <Tabs
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "All", count: items?.length },
            { value: "apps", label: "Apps", count: counts.apps },
            { value: "tasks", label: "Tasks", count: counts.tasks },
            { value: "services", label: "Services", count: counts.services },
          ]}
        />
      </div>

      {blocked && system && !system.elevated && (
        <div className="mx-7 mb-3 flex items-center gap-3 rounded-lg border border-line px-4 py-2.5">
          <ShieldHalf className="size-4 shrink-0 text-warn" />
          <span className="flex-1 text-[12.5px] text-ink-2">Items that start for everyone on this PC, and services, need administrator rights to change.</span>
          <Button
            size="sm"
            onClick={() =>
              api.relaunchAsAdmin().catch((e) => toast("Reclaim is still running without admin rights", { detail: errorMessage(e), tone: "warn" }))
            }
          >
            Restart as administrator
          </Button>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto border-t border-line px-7 pt-2 pb-8">
        {!items ? (
          <div className="flex justify-center py-20">
            <Spinner className="size-5" />
          </div>
        ) : matching.length === 0 ? (
          <EmptyState icon={<Rocket />} title={query ? `Nothing matches “${query}”` : "Nothing starts with Windows"}>
            {query ? "Try the app's name, its publisher, or part of its path." : "No apps, tasks, or services from your apps start automatically."}
          </EmptyState>
        ) : (
          shownGroups.map((g) => {
            const list = matching.filter((i) => groupOf(i) === g.id);
            if (list.length === 0) return null;
            return (
              <section key={g.id} className="mt-3 mb-4">
                <div className="mb-1.5 px-3">
                  <h2 className="text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">
                    {g.label} <span className="num ml-1 font-normal tracking-normal">{list.length}</span>
                  </h2>
                  <p className="mt-0.5 text-[12.5px] text-ink-3">{g.about}</p>
                </div>
                <ul className="overflow-hidden rounded-lg border border-line">
                  {list.map((item) => (
                    <StartupRow
                      key={item.id}
                      item={item}
                      busy={pending.has(item.id)}
                      onToggle={(on) => toggle(item, on)}
                      onReveal={() => item.program && api.revealPath(item.program).catch(() => {})}
                    />
                  ))}
                </ul>
              </section>
            );
          })
        )}
        {items && items.length > 0 && (
          <p className="px-3 text-[12px] text-ink-3">
            Turning something off never deletes it. Apps use the same switch as Task Manager, so the two always agree.
          </p>
        )}
      </div>
    </div>
  );
}

function StartupRow({
  item,
  busy,
  onToggle,
  onReveal,
}: {
  item: StartupItem;
  busy: boolean;
  onToggle: (on: boolean) => void;
  onReveal: () => void;
}) {
  const showEntry = item.entry.toLowerCase() !== item.name.toLowerCase();
  return (
    <li
      className={cx("group flex items-center gap-3.5 border-b border-line/60 px-4 py-2.5 last:border-b-0", !item.enabled && "opacity-75")}
      title={item.command}
    >
      <FileIcon path={item.missing ? null : item.program} name={item.name} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13.5px] text-ink">{item.name}</span>
          {item.missing && (
            <Badge tone="warn" icon={<TriangleAlert />}>
              Program not found
            </Badge>
          )}
        </div>
        <div className="truncate text-[12px] text-ink-3">
          {item.publisher ?? "Unknown publisher"}
          <span className="mx-1.5 text-ink-3/50">·</span>
          {whereFrom(item)}
          {showEntry && <span className="num ml-2 text-ink-3/70">{item.entry}</span>}
        </div>
      </div>
      {item.program && !item.missing && (
        <IconButton label="Show in folder" onClick={onReveal} className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100">
          <FolderOpen className="size-3.5" />
        </IconButton>
      )}
      {busy && <Spinner className="size-3.5" />}
      {item.blocked && (
        <span title="Needs administrator rights" className="text-ink-3">
          <Lock className="size-3.5" aria-label="Needs administrator rights" />
        </span>
      )}
      <Switch label={`Start ${item.name} with Windows`} checked={item.enabled} disabled={item.blocked || busy} onChange={onToggle} />
    </li>
  );
}
