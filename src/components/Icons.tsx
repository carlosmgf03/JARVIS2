// Iconos lineales mínimos (sin dependencias externas).
const base = { width: 20, height: 20, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const, 'aria-hidden': true };

export const MicIcon = () => (
  <svg {...base}>
    <rect x="9" y="3" width="6" height="11" rx="3" />
    <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
  </svg>
);
export const MicOffIcon = () => (
  <svg {...base}>
    <path d="M9 9V6a3 3 0 0 1 5.6-1.5M15 11v0a3 3 0 0 1-4.5 2.6M5 11a7 7 0 0 0 11.5 5.3M19 11a7 7 0 0 1-.6 2.8M12 18v3M3 3l18 18" />
  </svg>
);
export const StopIcon = () => (
  <svg {...base}>
    <rect x="7" y="7" width="10" height="10" rx="1.5" />
  </svg>
);
export const PowerIcon = () => (
  <svg {...base}>
    <path d="M12 3v8M7.5 6.5a7 7 0 1 0 9 0" />
  </svg>
);
export const SpeakerIcon = ({ muted }: { muted?: boolean }) => (
  <svg {...base}>
    <path d="M4 9h3l5-4v14l-5-4H4z" />
    {muted ? <path d="M16 9l5 6M21 9l-5 6" /> : <path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11" />}
  </svg>
);
export const PanelIcon = () => (
  <svg {...base}>
    <path d="M4 6h16M4 12h16M4 18h10" />
  </svg>
);
export const CloseIcon = () => (
  <svg {...base}>
    <path d="M6 6l12 12M18 6L6 18" />
  </svg>
);
