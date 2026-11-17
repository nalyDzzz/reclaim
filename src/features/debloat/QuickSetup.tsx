import { useEffect, useMemo, useState } from "react";
import { Check, Download, EyeOff, Feather, Gamepad2, Upload } from "lucide-react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type AppxApp, type Preset, type Tweak } from "../../lib/api";
import { plural } from "../../lib/format";
import { useApp } from "../../state/app";
import { useDebloat } from "../../state/debloat";
import { Button, Spinner } from "../../components/ui";
import { buildPlan, planSize, type Plan } from "./plan";
import { PlanDialog } from "./PlanDialog";

const ICONS: Record<string, React.ReactNode> = {
  essentials: <Feather />,
  privacy: <EyeOff />,
  gaming: <Gamepad2 />,
};

const SETUP_FILTER = [{ name: "Reclaim setup", extensions: ["json"] }];

export function QuickSetup() {
  const { system, toast } = useApp();
  const { revision } = useDebloat();
  const elevated = system?.elevated ?? false;
  const [presets, setPresets] = useState<Preset[] | null>(null);
  const [tweaks, setTweaks] = useState<Tweak[] | null>(null);
  const [apps, setApps] = useState<AppxApp[] | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [busy, setBusy] = useState<"save" | "load" | null>(null);

  useEffect(() => {
    api
      .listPresets()
      .then(setPresets)
      .catch(() => setPresets([]));
  }, []);

  useEffect(() => {
    api
      .listTweaks()
      .then(setTweaks)
      .catch(() => setTweaks([]));
    api
      .listAppx()
      .then(setApps)
      .catch(() => setApps([]));
  }, [revision]);

  const plans = useMemo(() => {
    if (!presets || !tweaks || !apps) return null;
    return presets.map((p) => ({
      preset: p,
      plan: buildPlan(
        { title: `Apply ${p.name}?`, intro: p.description, tweakIds: p.tweaks, app: (a) => p.removeApps && a.recommended },
        tweaks,
        apps,
        elevated,
      ),
    }));
  }, [presets, tweaks, apps, elevated]);

  const saveSetup = async () => {
    const path = await save({ defaultPath: "Reclaim setup.json", filters: SETUP_FILTER });
    if (!path) return;
    setBusy("save");
    try {
      const saved = await api.exportSetup(path);
      toast("Saved your setup", {
        detail: `${plural(saved.tweaks.length, "setting")} and ${plural(saved.apps.length, "removed app")}.`,
        tone: "good",
      });
    } catch (e) {
      toast("Couldn't save your setup", { detail: errorMessage(e), tone: "danger" });
    } finally {
      setBusy(null);
    }
  };

  const loadSetup = async () => {
    const path = await open({ multiple: false, directory: false, filters: SETUP_FILTER });
    if (!path || !tweaks || !apps) return;
    setBusy("load");
    try {
      const setup = await api.readSetup(path);
      const names = new Set(setup.apps.map((n) => n.toLowerCase()));
      const unknown = setup.unknownTweaks;
      setPlan(
        buildPlan(
          {
            title: "Load this setup?",
            intro: `From ${path.split(/[\\/]/).pop()}. Only what isn't already done on this PC is listed.`,
            tweakIds: setup.tweaks,
            app: (a) => names.has(a.name.toLowerCase()),
            note: unknown
              ? `${plural(unknown, "setting")} in the file ${unknown === 1 ? "isn't" : "aren't"} in this version of Windows, so ${unknown === 1 ? "it's" : "they're"} skipped.`
              : undefined,
          },
          tweaks,
          apps,
          elevated,
        ),
      );
    } catch (e) {
      toast("Couldn't load that setup", { detail: errorMessage(e), tone: "danger" });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-8">
      <p className="mb-4 max-w-2xl text-[13px] text-ink-2">
        Pick a starting point. You'll see every change before anything happens, and each one can be undone later from the other tabs.
      </p>

      <div className="grid grid-cols-[repeat(auto-fit,minmax(230px,1fr))] gap-3">
        {(plans ?? presets?.map((preset) => ({ preset, plan: null })) ?? []).map(({ preset, plan: presetPlan }) => {
          const changes = presetPlan ? planSize(presetPlan) : null;
          const skipped = presetPlan ? presetPlan.blockedTweaks.length + presetPlan.blockedCopies.length : 0;
          return (
            <article key={preset.id} className="flex flex-col rounded-lg border border-line bg-white/2 p-4">
              <div className="flex items-center gap-2.5">
                <span className="flex size-8 items-center justify-center rounded-md bg-uv-600/15 text-uv-300 [&_svg]:size-4">
                  {ICONS[preset.id]}
                </span>
                <h2 className="text-[14px] font-semibold text-ink">{preset.name}</h2>
              </div>
              <p className="mt-2.5 flex-1 text-[12.5px] leading-relaxed text-ink-2">{preset.description}</p>
              <div className="mt-4 flex items-center justify-between gap-3">
                <span className="text-[12px] text-ink-3">
                  {changes === null ? (
                    <Spinner className="size-3.5" />
                  ) : changes === 0 && skipped === 0 ? (
                    <span className="flex items-center gap-1.5 text-good">
                      <Check className="size-3.5" /> All done
                    </span>
                  ) : (
                    <>
                      <span className="num text-ink">{changes}</span> {changes === 1 ? "change" : "changes"}
                      {skipped > 0 && <> · {skipped} need admin</>}
                    </>
                  )}
                </span>
                <Button
                  size="sm"
                  variant={changes ? "primary" : "secondary"}
                  disabled={!presetPlan}
                  onClick={() => presetPlan && setPlan(presetPlan)}
                >
                  Review
                </Button>
              </div>
            </article>
          );
        })}
      </div>

      <section className="mt-8">
        <h2 className="mb-1 text-[12px] font-semibold tracking-[0.1em] text-ink-3 uppercase">Your setup on another PC</h2>
        <p className="max-w-2xl text-[12.5px] text-ink-2">
          Save the settings you have on and the Store apps you've removed to a file. Load it in Reclaim on another PC to do the same there,
          after reviewing it.
        </p>
        <div className="mt-3 flex gap-2">
          <Button icon={<Download className="size-3.5" />} busy={busy === "save"} onClick={saveSetup}>
            Save setup to a file
          </Button>
          <Button icon={<Upload className="size-3.5" />} busy={busy === "load"} disabled={!tweaks || !apps} onClick={loadSetup}>
            Load a setup file
          </Button>
        </div>
      </section>

      {plan && <PlanDialog plan={plan} onClose={() => setPlan(null)} />}
    </div>
  );
}
