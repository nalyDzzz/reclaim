/** A drive platter with a wedge of space taken back. */
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
      <path d="M12 3a9 9 0 1 0 9 9h-9z" fill="var(--color-uv-500)" />
      <path d="M14.5 1.5a8 8 0 0 1 8 8h-8z" fill="var(--color-uv-300)" />
      <circle cx="12" cy="12" r="2" fill="var(--color-backdrop)" />
    </svg>
  );
}
