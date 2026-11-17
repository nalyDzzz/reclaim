import { useState } from "react";
import { History, LogIn, Lock, RotateCcw, X } from "lucide-react";
import { api, errorMessage } from "../../lib/api";
import { plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { useDebloat } from "../../state/debloat";
import { Badge, Button, IconButton, PageHeader, Tabs } from "../../components/ui";
import { QuickSetup } from "./QuickSetup";
import { StoreApps } from "./StoreApps";
import { WindowsSettings } from "./WindowsSettings";

type Section = "setup" | "apps" | "settings";

export function DebloatView() {
  const [section, setSection] = useState<Section>("setup");
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
              { value: "setup", label: "Quick setup" },
              { value: "apps", label: "Store apps" },
              { value: "settings", label: "Windows settings" },
            ]}
          />
        }
      />
      <DriftBanner />
      <ExplorerRestartBanner />
      <SignInBanner />
      {section === "setup" ? <QuickSetup /> : section === "apps" ? <StoreApps /> : <WindowsSettings />}
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

function SignInBanner() {
  const { needsSignIn, setNeedsSignIn } = useDebloat();
  if (!needsSignIn) return null;
  return (
    <div className="mx-7 mb-4 flex items-center gap-3 rounded-lg border border-line px-4 py-2.5">
      <LogIn className="size-4 shrink-0 text-uv-300" />
      <span className="flex-1 text-[13px] text-ink">Some changes apply the next time you sign in to Windows.</span>
      <IconButton label="Dismiss" onClick={() => setNeedsSignIn(false)}>
        <X className="size-3.5" />
      </IconButton>
    </div>
  );
}
