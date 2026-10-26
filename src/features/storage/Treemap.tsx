import { useEffect, useMemo, useRef, useState } from "react";
import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from "d3-hierarchy";
import type { TreemapNode } from "../../lib/api";
import { formatBytes, formatPercent } from "../../lib/format";

interface Datum {
  id: number;
  name: string;
  size: number;
  isDir: boolean;
  /** Aggregate of items too small to draw on their own. */
  rest?: boolean;
  children?: Datum[];
}

/** Folder fills step lighter with depth; files are neutral so they read apart. */
const DEPTH_FILL = ["var(--color-uv-700)", "var(--color-uv-600)", "var(--color-uv-500)", "var(--color-uv-400)", "var(--color-uv-300)"];
const FILE_FILL = "#464a57";
const GROUP_FILL = "var(--color-uv-900)";
const HEADER = 17;

function toDatum(node: TreemapNode): Datum {
  const children = node.children.map(toDatum);
  if (children.length > 0 && node.other > 0) {
    children.push({ id: -node.id - 1, name: "Smaller items", size: node.other, isDir: false, rest: true });
  }
  return { id: node.id, name: node.name, size: node.size, isDir: node.isDir, children: children.length ? children : undefined };
}

export function Treemap({
  data,
  highlightId,
  onOpen,
  onHover,
}: {
  data: TreemapNode;
  highlightId: number | null;
  onOpen: (id: number) => void;
  onHover?: (id: number | null) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [hover, setHover] = useState<{ node: HierarchyRectangularNode<Datum>; x: number; y: number } | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize({ w: Math.floor(width), h: Math.floor(height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const nodes = useMemo(() => {
    if (size.w < 10 || size.h < 10 || data.size <= 0) return [];
    const root = hierarchy(toDatum(data))
      .sum((d) => (d.children ? 0 : d.size))
      .sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
    const fitsHeader = (d: HierarchyRectangularNode<Datum>) => d.depth > 0 && d.x1 - d.x0 > 56 && d.y1 - d.y0 > 44;
    const layout = treemap<Datum>()
      .tile(treemapSquarify.ratio(1.2))
      .size([size.w, size.h])
      .paddingInner(2)
      .paddingTop((d) => (fitsHeader(d) ? HEADER : d.depth > 0 ? 2 : 0))
      .paddingLeft((d) => (d.depth > 0 ? 2 : 0))
      .paddingRight((d) => (d.depth > 0 ? 2 : 0))
      .paddingBottom((d) => (d.depth > 0 ? 2 : 0))
      .round(true);
    return layout(root)
      .descendants()
      // Cells smaller than a few pixels can't be seen or hovered; skip them.
      .filter((d) => d.depth > 0 && d.x1 - d.x0 >= 3 && d.y1 - d.y0 >= 3);
  }, [data, size]);

  const total = data.size;

  return (
    <div
      ref={box}
      className="relative h-full w-full overflow-hidden"
      onMouseLeave={() => {
        setHover(null);
        onHover?.(null);
      }}
    >
      {nodes.map((d) => {
        const w = d.x1 - d.x0;
        const h = d.y1 - d.y0;
        const isGroup = !!d.children;
        const datum = d.data;
        const fill = datum.rest
          ? undefined
          : isGroup
            ? GROUP_FILL
            : datum.isDir
              ? DEPTH_FILL[Math.min(d.depth, DEPTH_FILL.length) - 1]
              : FILE_FILL;
        const highlighted = datum.id === highlightId || hover?.node === d;
        // The two lightest ramp steps need dark text to stay legible.
        const lightFill = !isGroup && datum.isDir && d.depth >= 4;
        return (
          <div
            key={`${datum.id}-${d.depth}`}
            className="absolute overflow-hidden rounded-[3px]"
            style={{
              left: d.x0,
              top: d.y0,
              width: w,
              height: h,
              background: datum.rest
                ? "repeating-linear-gradient(135deg, rgb(255 255 255 / 0.06) 0 2px, transparent 2px 6px)"
                : fill,
              boxShadow: highlighted ? "inset 0 0 0 1.5px var(--color-uv-300)" : undefined,
              cursor: datum.isDir && !datum.rest ? "pointer" : "default",
            }}
            onMouseMove={(e) => {
              const rect = box.current!.getBoundingClientRect();
              setHover({ node: d, x: e.clientX - rect.left, y: e.clientY - rect.top });
              onHover?.(datum.rest ? null : datum.id);
            }}
            onClick={() => {
              if (datum.isDir && !datum.rest) onOpen(datum.id);
            }}
          >
            {isGroup && h > 30 && w > 56 && (
              <div
                className="pointer-events-none flex h-[17px] items-center gap-1.5 truncate px-1.5 text-[11px] leading-none text-uv-300"
              >
                <span className="truncate font-medium text-ink">{datum.name}</span>
                <span className="num shrink-0 text-ink-3">{formatBytes(d.value ?? 0)}</span>
              </div>
            )}
            {!isGroup && w > 54 && h > 30 && (
              <div className={`pointer-events-none flex flex-col px-1.5 pt-1 leading-tight ${lightFill ? "text-backdrop" : "text-white"}`}>
                <span className="truncate text-[11.5px] font-medium">{datum.name}</span>
                {h > 42 && <span className="num truncate text-[10.5px] opacity-75">{formatBytes(d.value ?? 0)}</span>}
              </div>
            )}
          </div>
        );
      })}

      {hover && (
        <TreemapTooltip
          node={hover.node}
          x={hover.x}
          y={hover.y}
          containerWidth={size.w}
          containerHeight={size.h}
          total={total}
        />
      )}
    </div>
  );
}

function TreemapTooltip({
  node,
  x,
  y,
  containerWidth,
  containerHeight,
  total,
}: {
  node: HierarchyRectangularNode<Datum>;
  x: number;
  y: number;
  containerWidth: number;
  containerHeight: number;
  total: number;
}) {
  const path = node
    .ancestors()
    .reverse()
    .slice(1)
    .map((a) => a.data.name)
    .join("\\");
  const value = node.value ?? 0;
  const width = 260;
  const left = Math.min(x + 14, containerWidth - width - 8);
  const top = y + 90 > containerHeight ? y - 84 : y + 16;
  return (
    <div
      className="pointer-events-none absolute z-10 rounded-lg border border-line-strong bg-raised/95 px-3 py-2.5 shadow-xl shadow-black/40 backdrop-blur"
      style={{ left: Math.max(8, left), top, width }}
    >
      <div className="truncate text-[12.5px] font-medium text-ink">{node.data.rest ? "Smaller items" : node.data.name}</div>
      {!node.data.rest && path !== node.data.name && <div className="num mt-0.5 truncate text-[11px] text-ink-3">{path}</div>}
      <div className="mt-1.5 flex items-baseline justify-between gap-3">
        <span className="num text-[13px] text-ink">{formatBytes(value)}</span>
        <span className="num text-[11.5px] text-ink-2">{formatPercent(value, total)} of this folder</span>
      </div>
      {node.data.isDir && !node.data.rest && <div className="mt-1 text-[11px] text-ink-3">Click to open</div>}
    </div>
  );
}
