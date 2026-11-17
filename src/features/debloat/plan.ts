import type { AppxApp, Tweak } from "../../lib/api";

/** A batch of Debloat changes, shown in full before any of it happens. */
export interface Plan {
  title: string;
  intro: string;
  /** Settings to turn on that aren't on yet. */
  tweaks: Tweak[];
  /** Apps to remove: installed ones, and Windows' copies when running as administrator. */
  apps: AppxApp[];
  /** Settings that need administrator rights Reclaim doesn't have right now. */
  blockedTweaks: Tweak[];
  /** Copies Windows keeps, which only an administrator can remove. */
  blockedCopies: AppxApp[];
  /** Anything else worth saying, like settings in a file this PC doesn't have. */
  note?: string;
}

export function buildPlan(
  want: { title: string; intro: string; tweakIds: string[]; app: (a: AppxApp) => boolean; note?: string },
  tweaks: Tweak[],
  apps: AppxApp[],
  elevated: boolean,
): Plan {
  const ids = new Set(want.tweakIds);
  const toChange = tweaks.filter((t) => ids.has(t.id) && !t.applied);
  const chosen = apps.filter(want.app);
  const copies = chosen.filter((a) => !a.installed);
  return {
    title: want.title,
    intro: want.intro,
    note: want.note,
    tweaks: toChange.filter((t) => !t.blocked),
    blockedTweaks: toChange.filter((t) => t.blocked),
    apps: [...chosen.filter((a) => a.installed), ...(elevated ? copies : [])],
    blockedCopies: elevated ? [] : copies,
  };
}

/** How many changes applying the plan makes. */
export function planSize(plan: Plan): number {
  return plan.tweaks.length + plan.apps.length;
}
