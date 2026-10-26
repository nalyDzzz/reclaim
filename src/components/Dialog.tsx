import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { cx, IconButton } from "./ui";

/** Modal dialog. `onClose` is omitted while a step can't be interrupted. */
export function Dialog({
  title,
  icon,
  onClose,
  children,
  footer,
  width = 520,
}: {
  title: ReactNode;
  icon?: ReactNode;
  onClose?: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && onClose) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    // Move focus into the dialog so keyboard users land on its controls.
    const first = panel.current?.querySelector<HTMLElement>("[data-autofocus], button:not([disabled])");
    first?.focus();
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6 backdrop-blur-[2px]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && onClose) onClose();
      }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        className="animate-rise flex max-h-full w-full flex-col overflow-hidden rounded-xl border border-line-strong bg-raised shadow-2xl shadow-black/50"
        style={{ maxWidth: width }}
      >
        <div className="flex items-start gap-3 px-5 pt-5 pb-3">
          {icon && <div className="mt-0.5 shrink-0 [&_svg]:size-5">{icon}</div>}
          <h2 className="min-w-0 flex-1 text-[15px] leading-snug font-semibold text-ink">{title}</h2>
          {onClose && (
            <IconButton label="Close" onClick={onClose} className="-mt-1 -mr-2">
              <X className="size-4" />
            </IconButton>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5 text-[13px] text-ink-2">{children}</div>
        {footer && (
          <div className={cx("flex items-center justify-end gap-2 border-t border-line bg-panel/60 px-5 py-3")}>{footer}</div>
        )}
      </div>
    </div>
  );
}
