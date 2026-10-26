import { forwardRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from "react";
import { Check, LoaderCircle, Search, X } from "lucide-react";

export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const BUTTON_STYLES: Record<ButtonVariant, string> = {
  primary: "bg-uv-600 text-white hover:bg-uv-500 active:bg-uv-700 shadow-[inset_0_1px_0_rgb(255_255_255/0.14)]",
  secondary: "bg-raised text-ink border border-line-strong hover:bg-hover active:bg-raised",
  ghost: "text-ink-2 hover:text-ink hover:bg-hover",
  danger: "bg-danger/90 text-white hover:bg-danger active:bg-danger/80 shadow-[inset_0_1px_0_rgb(255_255_255/0.14)]",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "sm" | "md";
  icon?: ReactNode;
  busy?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, busy, disabled, className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      disabled={disabled || busy}
      className={cx(
        "inline-flex shrink-0 items-center justify-center gap-2 rounded-md font-medium whitespace-nowrap transition-colors",
        "disabled:pointer-events-none disabled:opacity-45",
        size === "sm" ? "h-7 px-2.5 text-[12.5px]" : "h-8.5 px-3.5 text-[13px]",
        BUTTON_STYLES[variant],
        className,
      )}
      {...rest}
    >
      {busy ? <LoaderCircle className="size-3.5 animate-spin" /> : icon}
      {children}
    </button>
  );
});

export function IconButton({
  label,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex size-7 shrink-0 items-center justify-center rounded-md text-ink-2 transition-colors hover:bg-hover hover:text-ink disabled:opacity-40",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Checkbox({
  checked,
  indeterminate,
  onChange,
  label,
  disabled,
  className,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  const on = checked || indeterminate;
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-label={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      className={cx(
        "inline-flex size-4 shrink-0 items-center justify-center rounded-[4px] border transition-colors disabled:opacity-40",
        on ? "border-uv-500 bg-uv-600 text-white" : "border-ink-3/70 bg-transparent hover:border-ink-2",
        className,
      )}
    >
      {indeterminate ? <span className="h-0.5 w-2 rounded bg-white" /> : checked && <Check className="size-3" strokeWidth={3} />}
    </button>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors disabled:opacity-40",
        checked ? "border-uv-500 bg-uv-600" : "border-ink-3/70 bg-transparent hover:border-ink-2",
      )}
    >
      <span
        className={cx(
          "absolute size-3 rounded-full transition-all",
          checked ? "left-[19px] size-3.5 bg-white" : "left-[4px] bg-ink-2",
        )}
      />
    </button>
  );
}

type BadgeTone = "neutral" | "uv" | "good" | "warn" | "danger";

const BADGE_STYLES: Record<BadgeTone, string> = {
  neutral: "bg-white/6 text-ink-2",
  uv: "bg-uv-600/20 text-uv-300",
  good: "bg-good/12 text-good",
  warn: "bg-warn/12 text-warn",
  danger: "bg-danger/12 text-danger",
};

export function Badge({ tone = "neutral", icon, children }: { tone?: BadgeTone; icon?: ReactNode; children: ReactNode }) {
  return (
    <span
      className={cx(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded px-1.5 text-[11.5px] font-medium whitespace-nowrap [&_svg]:size-3",
        BADGE_STYLES[tone],
      )}
    >
      {icon}
      {children}
    </span>
  );
}

export function Spinner({ className }: { className?: string }) {
  return <LoaderCircle className={cx("size-4 animate-spin text-ink-3", className)} />;
}

/** Horizontal fill bar. `fraction` is clamped to 0..1. */
export function Meter({ fraction, className, tone = "uv" }: { fraction: number; className?: string; tone?: "uv" | "warn" | "danger" }) {
  const pct = Math.max(0, Math.min(1, fraction)) * 100;
  const fill = tone === "danger" ? "bg-danger" : tone === "warn" ? "bg-warn" : "bg-uv-500";
  return (
    <div className={cx("h-1.5 overflow-hidden rounded-full bg-white/7", className)}>
      <div className={cx("h-full rounded-full transition-[width] duration-500", fill)} style={{ width: `${pct}%` }} />
    </div>
  );
}

export function SearchField({
  value,
  onChange,
  className,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, "onChange"> & { value: string; onChange: (v: string) => void }) {
  return (
    <label
      className={cx(
        "flex h-8.5 items-center gap-2 rounded-md border border-line-strong bg-backdrop/60 px-2.5 text-ink-2 focus-within:border-uv-500",
        className,
      )}
    >
      <Search className="size-3.5 shrink-0" />
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
        className="min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none placeholder:text-ink-3"
        {...rest}
      />
      {value && (
        <button type="button" aria-label="Clear search" onClick={() => onChange("")} className="text-ink-3 hover:text-ink">
          <X className="size-3.5" />
        </button>
      )}
    </label>
  );
}

export function Tabs<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string; count?: number }[];
}) {
  return (
    <div role="tablist" className="inline-flex gap-0.5 rounded-lg bg-backdrop/60 p-0.5 ring-1 ring-line">
      {options.map((o) => (
        <button
          key={o.value}
          role="tab"
          aria-selected={value === o.value}
          onClick={() => onChange(o.value)}
          className={cx(
            "flex h-7 items-center gap-1.5 rounded-md px-3 text-[12.5px] font-medium transition-colors",
            value === o.value ? "bg-raised text-ink shadow-sm ring-1 ring-line-strong" : "text-ink-2 hover:text-ink",
          )}
        >
          {o.label}
          {o.count !== undefined && <span className="num text-[11px] text-ink-3">{o.count}</span>}
        </button>
      ))}
    </div>
  );
}

export function EmptyState({ icon, title, children }: { icon: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-8 py-16 text-center">
      <div className="flex size-11 items-center justify-center rounded-xl bg-white/5 text-ink-2 [&_svg]:size-5">{icon}</div>
      <div className="text-[14px] font-semibold text-ink">{title}</div>
      {children && <div className="max-w-sm text-[13px] text-ink-2">{children}</div>}
    </div>
  );
}

/** Page header: wide display title with a supporting line and actions. */
export function PageHeader({ title, meta, actions }: { title: string; meta?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex items-end justify-between gap-6 px-7 pt-6 pb-4">
      <div className="min-w-0">
        <h1 className="display text-[26px] leading-none text-ink">{title}</h1>
        {meta && <div className="mt-2 text-[13px] text-ink-2">{meta}</div>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}
