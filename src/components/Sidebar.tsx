import { HardDrive, Package, ShieldCheck, ShieldHalf, SlidersHorizontal, Sparkles } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes } from "../lib/format";
import { useApp } from "../state/app";
import { useUpdates } from "../state/updates";
import { cx, Meter, Spinner } from "./ui";

export type Page = "apps" | "storage" | "cleanup" | "debloat";

const NAV: { page: Page; label: string; icon: React.ReactNode }[] = [
  { page: "apps", label: "Apps", icon: <Package /> },
  { page: "storage", label: "Storage", icon: <HardDrive /> },
  { page: "cleanup", label: "Cleanup", icon: <Sparkles /> },
  { page: "debloat", label: "Debloat", icon: <SlidersHorizontal /> },
];

export function Sidebar({ page, onNavigate }: { page: Page; onNavigate: (p: Page) => void }) {
  const { drives, system, toast } = useApp();
  const fixed = drives.filter((d) => d.kind === "fixed" && d.totalBytes > 0);

  return (
    <nav className="flex w-[212px] shrink-0 flex-col px-2.5 pb-3" aria-label="Main">
      <ul className="mt-1 flex flex-col gap-0.5">
        {NAV.map((item) => {
          const active = item.page === page;
          return (
            <li key={item.page}>
              <button
                onClick={() => onNavigate(item.page)}
                aria-current={active ? "page" : undefined}
                className={cx(
                  "relative flex h-9 w-full items-center gap-3 rounded-md px-3 text-[13.5px] transition-colors [&_svg]:size-[17px]",
                  active ? "bg-white/8 font-medium text-ink" : "text-ink-2 hover:bg-white/5 hover:text-ink",
                )}
              >
                {active && <span className="absolute top-2.5 bottom-2.5 left-0 w-[3px] rounded-full bg-uv-400" />}
                <span className={active ? "text-uv-300" : undefined}>{item.icon}</span>
                {item.label}
              </button>
            </li>
          );
        })}
      </ul>

      <div className="mt-auto flex flex-col gap-4">
        {fixed.length > 0 && (
          <section aria-label="Drives" className="flex flex-col gap-3 px-3">
            {fixed.map((d) => {
              const used = d.totalBytes - d.freeBytes;
              const fraction = used / d.totalBytes;
              return (
                <div key={d.letter} className="flex flex-col gap-1.5">
                  <div className="flex items-baseline justify-between gap-2 text-[12px]">
                    <span className="truncate text-ink-2">
                      <span className="num text-ink">{d.letter}:</span> {d.label || "Local Disk"}
                    </span>
                    <span className="num shrink-0 text-[11px] text-ink-3">{formatBytes(d.freeBytes)} free</span>
                  </div>
                  <Meter fraction={fraction} tone={fraction > 0.95 ? "danger" : fraction > 0.9 ? "warn" : "uv"} />
                </div>
              );
            })}
          </section>
        )}

        {system &&
          (system.elevated ? (
            <div className="flex items-center gap-2 px-3 text-[12px] text-ink-2">
              <ShieldCheck className="size-3.5 text-good" />
              Running as administrator
            </div>
          ) : (
            <button
              onClick={() =>
                api.relaunchAsAdmin().catch((e) => toast("Reclaim is still running without admin rights", { detail: errorMessage(e), tone: "warn" }))
              }
              className="group flex items-start gap-2.5 rounded-lg border border-line px-3 py-2.5 text-left transition-colors hover:border-uv-500/60 hover:bg-white/4"
            >
              <ShieldHalf className="mt-0.5 size-4 shrink-0 text-warn" />
              <span className="flex flex-col gap-0.5">
                <span className="text-[12.5px] font-medium text-ink">Restart as administrator</span>
                <span className="text-[11.5px] leading-snug text-ink-3">Needed for system cleanup, some tweaks, and full scans</span>
              </span>
            </button>
          ))}

        {system && <VersionLine version={system.appVersion} />}
      </div>
    </nav>
  );
}

/** Current version, with a manual check that turns into the update prompt. */
function VersionLine({ version }: { version: string }) {
  const { status, available, checkNow, openDialog } = useUpdates();
  const checking = status.kind === "checking";
  return (
    <div className="flex items-center justify-between gap-2 px-3 text-[11.5px] whitespace-nowrap text-ink-3">
      <span className="num">v{version}</span>
      {available ? (
        <button onClick={openDialog} className="font-medium text-uv-300 hover:underline">
          Update available
        </button>
      ) : (
        <button onClick={checkNow} disabled={checking} className="flex items-center gap-1.5 hover:text-ink disabled:opacity-70">
          {checking && <Spinner className="size-3" />}
          {checking ? "Checking" : "Check for updates"}
        </button>
      )}
    </div>
  );
}
