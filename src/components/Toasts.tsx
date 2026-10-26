import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { useApp, type ToastTone } from "../state/app";
import { cx } from "./ui";

const ICONS: Record<ToastTone, React.ReactNode> = {
  neutral: <Info className="text-ink-2" />,
  good: <CircleCheck className="text-good" />,
  warn: <TriangleAlert className="text-warn" />,
  danger: <CircleAlert className="text-danger" />,
};

export function Toasts() {
  const { toasts, dismissToast } = useApp();
  return (
    <div className="pointer-events-none fixed right-5 bottom-5 z-[60] flex w-[360px] flex-col gap-2" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={cx(
            "animate-rise pointer-events-auto flex items-start gap-3 rounded-lg border border-line-strong bg-raised px-3.5 py-3 shadow-xl shadow-black/40",
          )}
        >
          <span className="mt-px shrink-0 [&_svg]:size-4">{ICONS[t.tone]}</span>
          <div className="min-w-0 flex-1">
            <div className="text-[13px] font-medium text-ink">{t.message}</div>
            {t.detail && <div className="mt-0.5 text-[12.5px] break-words text-ink-2">{t.detail}</div>}
          </div>
          <button aria-label="Dismiss" onClick={() => dismissToast(t.id)} className="text-ink-3 hover:text-ink">
            <X className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
