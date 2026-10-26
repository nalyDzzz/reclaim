import { useId } from "react";

/** A drive platter with a wedge of space taken back. Matches app-icon.svg. */
export function Logo({ className }: { className?: string }) {
  const gradient = useId();
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
      <defs>
        <linearGradient id={gradient} x1="4" y1="4" x2="19" y2="21" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#9a93ec" />
          <stop offset="1" stopColor="#5546bb" />
        </linearGradient>
      </defs>
      <g transform="translate(-0.75 0.75)">
        <path d="M12 3A9 9 0 1 0 21 12H14A2 2 0 1 1 12 10Z" fill={`url(#${gradient})`} />
        <path d="M14.5 1.5a8 8 0 0 1 8 8h-8z" fill="#c9c5fb" />
      </g>
    </svg>
  );
}
