import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useApp } from "../state/app";
import { formatBytes } from "../lib/format";
import { cx } from "./ui";
import { Logo } from "./Logo";

const appWindow = getCurrentWindow();

export function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const sync = () => appWindow.isMaximized().then(setMaximized).catch(() => {});
    sync();
    const unlisten = appWindow.onResized(sync);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <header data-tauri-drag-region className="flex h-11 shrink-0 items-center pl-4">
      <div data-tauri-drag-region className="flex items-center gap-2.5">
        <Logo className="pointer-events-none size-[18px]" />
        <span data-tauri-drag-region className="display text-[13px] tracking-wide text-ink">
          Reclaim
        </span>
      </div>
      <div data-tauri-drag-region className="h-full flex-1" />
      <ReclaimedLedger />
      <div className="ml-3 flex h-full">
        <WindowButton label="Minimize" onClick={() => appWindow.minimize()}>
          <path d="M1 5.5h9" />
        </WindowButton>
        <WindowButton label={maximized ? "Restore" : "Maximize"} onClick={() => appWindow.toggleMaximize()}>
          {maximized ? (
            <>
              <rect x="1" y="3" width="7" height="7" rx="1" />
              <path d="M3 3V2a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H8" />
            </>
          ) : (
            <rect x="1" y="1" width="9" height="9" rx="1.2" />
          )}
        </WindowButton>
        <WindowButton label="Close" danger onClick={() => appWindow.close()}>
          <path d="M1 1l9 9M10 1l-9 9" />
        </WindowButton>
      </div>
    </header>
  );
}

function WindowButton({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cx(
        "flex h-full w-[46px] items-center justify-center text-ink-2 transition-colors",
        danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-white/8 hover:text-ink",
      )}
    >
      <svg viewBox="0 0 11 11" className="size-[11px]" fill="none" stroke="currentColor" strokeWidth="1">
        {children}
      </svg>
    </button>
  );
}

/**
 * The running total of space freed this session. Every tool reports into
 * it, and the digits roll when it grows.
 */
function ReclaimedLedger() {
  const { reclaimed } = useApp();
  const [pulse, setPulse] = useState(false);
  const previous = useRef(reclaimed);

  useEffect(() => {
    if (reclaimed > previous.current) {
      setPulse(true);
      const t = window.setTimeout(() => setPulse(false), 1400);
      previous.current = reclaimed;
      return () => window.clearTimeout(t);
    }
    previous.current = reclaimed;
  }, [reclaimed]);

  const [value, unit] = formatBytes(reclaimed).split(" ");
  return (
    <div
      data-tauri-drag-region
      title="Space freed this session by uninstalls, cleanup, and deletions"
      className={cx(
        "flex h-7 items-center gap-2.5 rounded-full border px-3 transition-[border-color,box-shadow] duration-700",
        pulse ? "border-uv-400/70 shadow-[0_0_18px_-2px_var(--color-uv-500)]" : "border-line-strong",
      )}
    >
      <span data-tauri-drag-region className="text-[10.5px] font-semibold tracking-[0.12em] text-ink-3 uppercase">
        Reclaimed
      </span>
      <span data-tauri-drag-region className={cx("num flex items-baseline gap-1 text-[13px]", reclaimed > 0 ? "text-ink" : "text-ink-3")}>
        <RollingNumber text={value} />
        <span className="text-[11px] text-ink-2">{unit}</span>
      </span>
    </div>
  );
}

function RollingNumber({ text }: { text: string }) {
  const chars = text.split("");
  return (
    <span className="inline-flex overflow-hidden" style={{ height: "1.2em", lineHeight: "1.2em" }}>
      {chars.map((ch, i) => {
        const key = chars.length - i; // stable from the right so columns don't remount
        if (!/\d/.test(ch)) {
          return <span key={key}>{ch}</span>;
        }
        return (
          <span key={key} className="relative inline-block w-[1ch]">
            <span
              className="absolute top-0 left-0 flex flex-col transition-transform duration-700 ease-[cubic-bezier(0.2,0.8,0.2,1)]"
              style={{ transform: `translateY(-${Number(ch) * 1.2}em)` }}
            >
              {"0123456789".split("").map((d) => (
                <span key={d} style={{ height: "1.2em" }}>
                  {d}
                </span>
              ))}
            </span>
            <span className="invisible">0</span>
          </span>
        );
      })}
    </span>
  );
}
