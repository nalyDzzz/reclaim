import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, PackageOpen, RefreshCw } from "lucide-react";
import { api, errorMessage, type Program } from "../../lib/api";
import { formatBytes, formatDate, plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { cx, EmptyState, IconButton, PageHeader, SearchField, Spinner, Tabs } from "../../components/ui";
import { ProgramIcon } from "./ProgramIcon";
import { ProgramDetail } from "./ProgramDetail";
import { UninstallDialog } from "./UninstallDialog";

type SortKey = "name" | "size" | "date" | "publisher";
type Filter = "all" | "recent";

const RECENT_DAYS = 30;

/** Measured folder size when known, else what the installer reported. */
export function programSize(p: Program, measured: Record<string, number>): number | null {
  const m = measured[p.id];
  if (m && m > 0) return m;
  return p.estimatedSize;
}

export function AppsView() {
  const { toast } = useApp();
  const [programs, setPrograms] = useState<Program[] | null>(null);
  const [measured, setMeasured] = useState<Record<string, number>>({});
  const [measuring, setMeasuring] = useState(false);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "size", desc: true });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [uninstalling, setUninstalling] = useState<Program | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await api.listPrograms();
      setPrograms(list);
      const items = list
        .map((p) => ({ id: p.id, path: p.inferredLocation ?? p.installLocation ?? "" }))
        .filter((i) => i.path);
      setMeasuring(true);
      api
        .measurePrograms(items)
        .then(setMeasured)
        .catch(() => {})
        .finally(() => setMeasuring(false));
    } catch (e) {
      toast("Couldn't read installed apps", { detail: errorMessage(e), tone: "danger" });
      setPrograms([]);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  const recentCutoff = useMemo(() => {
    const d = new Date();
    d.setDate(d.getDate() - RECENT_DAYS);
    return d.toISOString().slice(0, 10);
  }, []);

  const visible = useMemo(() => {
    if (!programs) return [];
    const q = deferredQuery.trim().toLowerCase();
    const list = programs.filter((p) => {
      if (filter === "recent" && (!p.installDate || p.installDate < recentCutoff)) return false;
      if (!q) return true;
      return p.name.toLowerCase().includes(q) || (p.publisher ?? "").toLowerCase().includes(q);
    });
    const dir = sort.desc ? -1 : 1;
    list.sort((a, b) => {
      switch (sort.key) {
        case "size":
          return ((programSize(a, measured) ?? -1) - (programSize(b, measured) ?? -1)) * dir;
        case "date":
          return (a.installDate ?? "").localeCompare(b.installDate ?? "") * dir;
        case "publisher":
          return (a.publisher ?? "~").localeCompare(b.publisher ?? "~") * dir || a.name.localeCompare(b.name);
        default:
          return a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) * dir;
      }
    });
    return list;
  }, [programs, deferredQuery, filter, sort, measured, recentCutoff]);

  const totalSize = useMemo(
    () => (programs ?? []).reduce((sum, p) => sum + (programSize(p, measured) ?? 0), 0),
    [programs, measured],
  );
  const largest = useMemo(() => Math.max(1, ...visible.map((p) => programSize(p, measured) ?? 0)), [visible, measured]);
  const recentCount = useMemo(
    () => (programs ?? []).filter((p) => p.installDate && p.installDate >= recentCutoff).length,
    [programs, recentCutoff],
  );
  const selected = programs?.find((p) => p.id === selectedId) ?? null;
  // The detail panel takes width, so the version column steps aside.
  const columns = selected ? "grid grid-cols-[minmax(0,1fr)_110px_170px]" : "grid grid-cols-[minmax(0,1fr)_120px_120px_190px]";

  const toggleSort = (key: SortKey) =>
    setSort((s) => (s.key === key ? { key, desc: !s.desc } : { key, desc: key === "size" || key === "date" }));

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <PageHeader
          title="Apps"
          meta={
            programs ? (
              <span className="flex items-center gap-2">
                {plural(programs.length, "app")} installed, using <span className="num text-ink">{formatBytes(totalSize)}</span>
                {measuring && (
                  <span className="flex items-center gap-1.5 text-ink-3">
                    <Spinner className="size-3" /> measuring folders
                  </span>
                )}
              </span>
            ) : (
              "Reading the list of installed apps…"
            )
          }
          actions={
            <IconButton label="Refresh list" onClick={() => load()}>
              <RefreshCw className="size-4" />
            </IconButton>
          }
        />

        <div className="flex items-center gap-3 px-7 pb-3">
          <SearchField value={query} onChange={setQuery} placeholder="Search apps or publishers" className="w-72" />
          <Tabs
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All", count: programs?.length },
              { value: "recent", label: `Last ${RECENT_DAYS} days`, count: recentCount },
            ]}
          />
        </div>

        <div className={cx(columns, "items-center gap-4 border-y border-line px-7 py-2 text-[11.5px] font-medium text-ink-3")}>
          <SortHeader label="Name" active={sort.key === "name"} desc={sort.desc} onClick={() => toggleSort("name")} />
          {!selected && <SortHeader label="Version" />}
          <SortHeader label="Installed" active={sort.key === "date"} desc={sort.desc} onClick={() => toggleSort("date")} />
          <SortHeader label="Size" align="right" active={sort.key === "size"} desc={sort.desc} onClick={() => toggleSort("size")} />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto" role="listbox" aria-label="Installed apps">
          {!programs ? (
            <div className="flex justify-center py-20">
              <Spinner className="size-5" />
            </div>
          ) : visible.length === 0 ? (
            <EmptyState icon={<PackageOpen />} title={query ? `No apps match “${query}”` : "No apps here"}>
              {query ? "Try the publisher's name, or part of the app's name." : "Nothing was installed in this period."}
            </EmptyState>
          ) : (
            visible.map((p) => {
              const size = programSize(p, measured);
              const isSelected = p.id === selectedId;
              return (
                <div
                  key={p.id}
                  role="option"
                  aria-selected={isSelected}
                  tabIndex={0}
                  onClick={() => setSelectedId(isSelected ? null : p.id)}
                  onDoubleClick={() => setUninstalling(p)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") setSelectedId(p.id);
                    if (e.key === "Delete") setUninstalling(p);
                  }}
                  className={cx(
                    columns,
                    "cursor-default items-center gap-4 border-b border-line/60 px-7 py-2.5 outline-none",
                    isSelected ? "bg-uv-600/14" : "hover:bg-white/3 focus-visible:bg-white/4",
                  )}
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <ProgramIcon id={p.id} name={p.name} />
                    <div className="min-w-0">
                      <div className="truncate text-[13.5px] text-ink">{p.name}</div>
                      <div className="truncate text-[12px] text-ink-3">{p.publisher ?? "Unknown publisher"}</div>
                    </div>
                  </div>
                  {!selected && (
                    <div className="num truncate text-[12px] text-ink-2" title={p.version ?? undefined}>
                      {p.version ?? "—"}
                    </div>
                  )}
                  <div className="text-[12.5px] text-ink-2">{formatDate(p.installDate)}</div>
                  <div className="flex items-center justify-end gap-3">
                    <div className="h-1 w-16 overflow-hidden rounded-full bg-white/6">
                      {size !== null && (
                        <div className="h-full rounded-full bg-uv-500" style={{ width: `${Math.max(2, (size / largest) * 100)}%` }} />
                      )}
                    </div>
                    <span className="num w-[72px] text-right text-[12.5px] text-ink">{size !== null ? formatBytes(size) : "—"}</span>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {selected && (
        <ProgramDetail
          program={selected}
          size={programSize(selected, measured)}
          sizeMeasured={(measured[selected.id] ?? 0) > 0}
          onClose={() => setSelectedId(null)}
          onUninstall={() => setUninstalling(selected)}
        />
      )}

      {uninstalling && (
        <UninstallDialog
          program={uninstalling}
          size={programSize(uninstalling, measured)}
          onClose={(changed) => {
            setUninstalling(null);
            if (changed) {
              setSelectedId(null);
              load();
            }
          }}
        />
      )}
    </div>
  );
}

function SortHeader({
  label,
  active,
  desc,
  align,
  onClick,
}: {
  label: string;
  active?: boolean;
  desc?: boolean;
  align?: "right";
  onClick?: () => void;
}) {
  const Arrow = desc ? ArrowDown : ArrowUp;
  if (!onClick) return <div className={align === "right" ? "text-right" : undefined}>{label}</div>;
  return (
    <button
      onClick={onClick}
      className={cx("flex items-center gap-1 hover:text-ink", align === "right" && "justify-end", active && "text-ink-2")}
    >
      {label}
      {active && <Arrow className="size-3" />}
    </button>
  );
}

