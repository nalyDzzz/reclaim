import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { cx } from "../../components/ui";

const cache = new Map<string, Promise<string | null>>();

function loadIcon(id: string): Promise<string | null> {
  let pending = cache.get(id);
  if (!pending) {
    pending = api.programIcon(id).catch(() => null);
    cache.set(id, pending);
  }
  return pending;
}

/** The program's own icon, or a monogram tile when it doesn't ship one. */
export function ProgramIcon({ id, name, size = 28 }: { id: string; name: string; size?: number }) {
  const [src, setSrc] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    loadIcon(id).then((url) => alive && setSrc(url));
    return () => {
      alive = false;
    };
  }, [id]);

  if (src) {
    return <img src={src} alt="" width={size} height={size} className="shrink-0 object-contain" draggable={false} />;
  }
  const letter = name.replace(/[^A-Za-z0-9]/g, "").charAt(0).toUpperCase() || "?";
  return (
    <span
      aria-hidden="true"
      className={cx(
        "flex shrink-0 items-center justify-center rounded-md bg-white/6 font-semibold text-ink-2 ring-1 ring-line",
        src === undefined && "opacity-0",
      )}
      style={{ width: size, height: size, fontSize: size * 0.45 }}
    >
      {letter}
    </span>
  );
}
