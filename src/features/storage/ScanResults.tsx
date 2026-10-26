import { useEffect, useMemo, useState } from "react";
import { ArrowUp, ChevronRight, File, FolderClosed, FolderOpen, Lock, RefreshCw, Trash2, X } from "lucide-react";
import {
  api,
  errorMessage,
  type FileCategory,
  type FileItem,
  type NodeDetails,
  type NodeItem,
  type ScanSummary,
  type TreemapNode,
  type TypeStat,
} from "../../lib/api";
import { formatBytes, formatCount, formatDate, formatDuration, formatPercent, plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { cx, IconButton, Spinner, Tabs } from "../../components/ui";
import { CATEGORIES } from "./categories";
import { DeleteDialog, type DeleteTarget } from "./DeleteDialog";
import { Treemap } from "./Treemap";

type Tab = "folders" | "files" | "types";

export function ScanResults({ summary, onRescan, onClose }: { summary: ScanSummary; onRescan: () => void; onClose: () => void }) {
  const { system, toast } = useApp();
  const [tab, setTab] = useState<Tab>("folders");
  const [nodeId, setNodeId] = useState(summary.rootId);
  const [version, setVersion] = useState(0);
  const [details, setDetails] = useState<NodeDetails | null>(null);
  const [map, setMap] = useState<TreemapNode | null>(null);
  const [hoverId, setHoverId] = useState<number | null>(null);
  const [deleting, setDeleting] = useState<DeleteTarget | null>(null);

  useEffect(() => {
    let alive = true;
    Promise.all([api.scanNode(nodeId, 400), api.scanTreemap(nodeId, 5)])
      .then(([d, m]) => {
        if (!alive) return;
        setDetails(d);
        setMap(m);
      })
      .catch((e) => {
        if (!alive) return;
        // The folder may have been deleted; fall back to the root.
        if (nodeId !== summary.rootId) setNodeId(summary.rootId);
        else toast("Couldn't load scan results", { detail: errorMessage(e), tone: "danger" });
      });
    return () => {
      alive = false;
    };
  }, [nodeId, version, summary.rootId, toast]);

  const onDeleted = (deleted: boolean) => {
    setDeleting(null);
    if (deleted) setVersion((v) => v + 1);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-start justify-between gap-6 px-7 pt-5 pb-3">
        <div className="min-w-0">
          <nav aria-label="Folder path" className="flex min-w-0 flex-wrap items-center gap-0.5 text-[13px]">
            {details?.breadcrumbs.map((c, i) => {
              const last = i === details.breadcrumbs.length - 1;
              return (
                <span key={c.id} className="flex min-w-0 items-center gap-0.5">
                  {i > 0 && <ChevronRight className="size-3.5 shrink-0 text-ink-3" />}
                  <button
                    onClick={() => setNodeId(c.id)}
                    disabled={last}
                    className={cx("truncate rounded px-1 py-0.5", last ? "font-medium text-ink" : "text-ink-2 hover:bg-hover hover:text-ink")}
                  >
                    {c.name}
                  </button>
                </span>
              );
            })}
          </nav>
          <div className="mt-1 flex items-baseline gap-3 px-1">
            <span className="display text-[26px] leading-none text-ink">{formatBytes(details?.node.size ?? summary.totalBytes)}</span>
            <span className="text-[12.5px] text-ink-2">
              {plural(details?.node.files ?? summary.files, "file")}
              {details && details.node.id !== summary.rootId && (
                <> · {formatPercent(details.node.size, summary.totalBytes)} of the scan</>
              )}
            </span>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <span className="mr-2 text-[12px] text-ink-3">
            Scanned {formatCount(summary.files)} files in {formatDuration(summary.durationMs)}
          </span>
          <IconButton label="Scan again" onClick={onRescan}>
            <RefreshCw className="size-4" />
          </IconButton>
          <IconButton label="Close scan" onClick={onClose}>
            <X className="size-4" />
          </IconButton>
        </div>
      </header>

      <div className="flex items-center justify-between gap-4 px-7 pb-3">
        <Tabs
          value={tab}
          onChange={setTab}
          options={[
            { value: "folders", label: "Folders" },
            { value: "files", label: "Largest files" },
            { value: "types", label: "File types" },
          ]}
        />
        {summary.errors > 0 && (
          <span className="flex items-center gap-1.5 text-[12px] text-ink-3">
            <Lock className="size-3.5" />
            {plural(summary.errors, "folder")} couldn't be read
            {!system?.elevated && " without administrator rights"}
          </span>
        )}
      </div>

      {tab === "folders" && (
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_minmax(380px,40%)] border-t border-line">
          <div className="min-h-0 p-3">
            {map ? (
              <Treemap data={map} highlightId={hoverId} onOpen={setNodeId} onHover={setHoverId} />
            ) : (
              <div className="flex h-full items-center justify-center">
                <Spinner />
              </div>
            )}
          </div>
          <FolderList
            details={details}
            hoverId={hoverId}
            onHover={setHoverId}
            onOpen={setNodeId}
            onDelete={setDeleting}
          />
        </div>
      )}
      {tab === "files" && <LargestFiles version={version} onDelete={setDeleting} />}
      {tab === "types" && <FileTypes version={version} />}

      {deleting && <DeleteDialog target={deleting} onClose={onDeleted} />}
    </div>
  );
}

function FolderList({
  details,
  hoverId,
  onHover,
  onOpen,
  onDelete,
}: {
  details: NodeDetails | null;
  hoverId: number | null;
  onHover: (id: number | null) => void;
  onOpen: (id: number) => void;
  onDelete: (t: DeleteTarget) => void;
}) {
  if (!details) return <div className="border-l border-line" />;
  const parent = details.breadcrumbs.length > 1 ? details.breadcrumbs[details.breadcrumbs.length - 2] : null;
  const total = details.node.size;
  const childPath = (item: NodeItem) => `${details.path.replace(/\\$/, "")}\\${item.name}`;

  return (
    <div className="flex min-h-0 flex-col border-l border-line">
      <div className="grid grid-cols-[minmax(0,1fr)_64px_84px_56px] gap-3 border-b border-line px-4 py-2 text-[11.5px] font-medium text-ink-3">
        <span>Name</span>
        <span className="text-right">Share</span>
        <span className="text-right">Size</span>
        <span />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto" onMouseLeave={() => onHover(null)}>
        {parent && (
          <button
            onClick={() => onOpen(parent.id)}
            className="flex w-full items-center gap-2.5 border-b border-line/60 px-4 py-2 text-left text-[13px] text-ink-2 hover:bg-white/3 hover:text-ink"
          >
            <ArrowUp className="size-4" /> Up to {parent.name}
          </button>
        )}
        {details.children.map((item) => {
          const share = total > 0 ? item.size / total : 0;
          return (
            <div
              key={item.id}
              onMouseEnter={() => onHover(item.id)}
              onClick={() => item.isDir && onOpen(item.id)}
              className={cx(
                "group grid cursor-default grid-cols-[minmax(0,1fr)_64px_84px_56px] items-center gap-3 border-b border-line/60 px-4 py-[7px]",
                hoverId === item.id ? "bg-uv-600/12" : "hover:bg-white/3",
              )}
            >
              <div className="flex min-w-0 items-center gap-2.5">
                <span className={cx("shrink-0 [&_svg]:size-4", item.isDir ? "text-uv-400" : "text-ink-3")}>
                  {item.isDir ? <FolderClosed /> : <File />}
                </span>
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-[13px] text-ink">{item.name}</span>
                    {item.inaccessible && <Lock className="size-3 shrink-0 text-ink-3" aria-label="Couldn't be read" />}
                  </div>
                  <div className="mt-1 h-[3px] overflow-hidden rounded-full bg-white/5">
                    <div className="h-full rounded-full bg-uv-500" style={{ width: `${share * 100}%` }} />
                  </div>
                </div>
              </div>
              <span className="num text-right text-[12px] text-ink-2">{formatPercent(item.size, total)}</span>
              <span className="num text-right text-[12.5px] text-ink">{formatBytes(item.size)}</span>
              <span className="flex justify-end gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                <IconButton
                  label="Show in File Explorer"
                  onClick={(e) => {
                    e.stopPropagation();
                    api.revealPath(childPath(item));
                  }}
                >
                  <FolderOpen className="size-3.5" />
                </IconButton>
                <IconButton
                  label="Delete"
                  className="hover:text-danger"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete({ id: item.id, name: item.name, size: item.size, isDir: item.isDir });
                  }}
                >
                  <Trash2 className="size-3.5" />
                </IconButton>
              </span>
            </div>
          );
        })}
        {details.hiddenCount > 0 && (
          <div className="px-4 py-2.5 text-[12px] text-ink-3">
            And {plural(details.hiddenCount, "smaller item")} totaling {formatBytes(details.hiddenBytes)}
          </div>
        )}
        {details.children.length === 0 && <div className="px-4 py-10 text-center text-[13px] text-ink-3">This folder is empty.</div>}
      </div>
    </div>
  );
}

function LargestFiles({ version, onDelete }: { version: number; onDelete: (t: DeleteTarget) => void }) {
  const [files, setFiles] = useState<FileItem[] | null>(null);

  useEffect(() => {
    api.scanLargestFiles(300).then(setFiles).catch(() => setFiles([]));
  }, [version]);

  if (!files) {
    return (
      <div className="flex flex-1 items-center justify-center border-t border-line">
        <Spinner />
      </div>
    );
  }
  const largest = files[0]?.size ?? 1;
  return (
    <div className="flex min-h-0 flex-1 flex-col border-t border-line">
      <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_110px_150px_56px] gap-4 border-b border-line px-7 py-2 text-[11.5px] font-medium text-ink-3">
        <span>Name</span>
        <span>Folder</span>
        <span>Modified</span>
        <span className="text-right">Size</span>
        <span />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {files.map((f) => {
          const parentPath = f.path.slice(0, f.path.length - f.name.length - 1);
          const folder = parentPath.endsWith(":") ? `${parentPath}\\` : parentPath;
          return (
            <div
              key={f.id}
              className="group grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_110px_150px_56px] items-center gap-4 border-b border-line/60 px-7 py-2 hover:bg-white/3"
            >
              <div className="flex min-w-0 items-center gap-2.5">
                <span className="shrink-0 text-ink-3 [&_svg]:size-4" title={CATEGORIES[f.category].label}>
                  {CATEGORIES[f.category].icon}
                </span>
                <span className="truncate text-[13px] text-ink" title={f.name}>
                  {f.name}
                </span>
              </div>
              <span className="num truncate text-[11.5px] text-ink-3" title={folder}>
                {folder}
              </span>
              <span className="text-[12px] text-ink-2">{formatDate(f.modified)}</span>
              <div className="flex items-center justify-end gap-3">
                <div className="h-1 w-14 overflow-hidden rounded-full bg-white/6">
                  <div className="h-full rounded-full bg-uv-500" style={{ width: `${(f.size / largest) * 100}%` }} />
                </div>
                <span className="num w-[68px] text-right text-[12.5px] text-ink">{formatBytes(f.size)}</span>
              </div>
              <span className="flex justify-end gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                <IconButton label="Show in File Explorer" onClick={() => api.revealPath(f.path)}>
                  <FolderOpen className="size-3.5" />
                </IconButton>
                <IconButton
                  label="Delete"
                  className="hover:text-danger"
                  onClick={() => onDelete({ id: f.id, name: f.name, size: f.size, isDir: false })}
                >
                  <Trash2 className="size-3.5" />
                </IconButton>
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function FileTypes({ version }: { version: number }) {
  const [types, setTypes] = useState<TypeStat[] | null>(null);

  useEffect(() => {
    api.scanFileTypes().then(setTypes).catch(() => setTypes([]));
  }, [version]);

  const categories = useMemo(() => {
    const map = new Map<FileCategory, { bytes: number; count: number }>();
    for (const t of types ?? []) {
      const agg = map.get(t.category) ?? { bytes: 0, count: 0 };
      agg.bytes += t.bytes;
      agg.count += t.count;
      map.set(t.category, agg);
    }
    return [...map.entries()].sort((a, b) => b[1].bytes - a[1].bytes);
  }, [types]);

  if (!types) {
    return (
      <div className="flex flex-1 items-center justify-center border-t border-line">
        <Spinner />
      </div>
    );
  }
  const total = categories.reduce((s, [, c]) => s + c.bytes, 0);
  const largestCategory = categories[0]?.[1].bytes ?? 1;
  const topTypes = types.slice(0, 60);
  const largestType = topTypes[0]?.bytes ?? 1;

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_minmax(0,1fr)] border-t border-line">
      <section aria-label="By category" className="min-h-0 overflow-y-auto px-7 py-5">
        <h3 className="mb-4 text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">By kind</h3>
        <ul className="flex flex-col gap-4">
          {categories.map(([cat, agg]) => (
            <li key={cat}>
              <div className="flex items-center gap-2.5">
                <span className="text-ink-2 [&_svg]:size-4">{CATEGORIES[cat].icon}</span>
                <span className="flex-1 text-[13px] text-ink">{CATEGORIES[cat].label}</span>
                <span className="num text-[12px] text-ink-3">{formatPercent(agg.bytes, total)}</span>
                <span className="num w-[76px] text-right text-[13px] text-ink">{formatBytes(agg.bytes)}</span>
              </div>
              <div className="mt-1.5 ml-[26px] h-2 overflow-hidden rounded-[3px] bg-white/5">
                <div className="h-full rounded-[3px] bg-uv-500" style={{ width: `${(agg.bytes / largestCategory) * 100}%` }} />
              </div>
              <div className="mt-1 ml-[26px] text-[11.5px] text-ink-3">{plural(agg.count, "file")}</div>
            </li>
          ))}
        </ul>
      </section>
      <section aria-label="By extension" className="flex min-h-0 flex-col border-l border-line">
        <div className="grid grid-cols-[minmax(0,1fr)_90px_110px] gap-3 border-b border-line px-5 py-2 text-[11.5px] font-medium text-ink-3">
          <span>Extension</span>
          <span className="text-right">Files</span>
          <span className="text-right">Size</span>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {topTypes.map((t) => (
            <div key={t.extension} className="grid grid-cols-[minmax(0,1fr)_90px_110px] items-center gap-3 border-b border-line/60 px-5 py-2">
              <div className="flex min-w-0 items-center gap-2.5">
                <span className="num w-16 shrink-0 truncate text-[12.5px] text-ink">{t.extension ? `.${t.extension}` : "(none)"}</span>
                <div className="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-white/5">
                  <div className="h-full rounded-full bg-uv-500" style={{ width: `${(t.bytes / largestType) * 100}%` }} />
                </div>
              </div>
              <span className="num text-right text-[12px] text-ink-2">{formatCount(t.count)}</span>
              <span className="num text-right text-[12.5px] text-ink">{formatBytes(t.bytes)}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

