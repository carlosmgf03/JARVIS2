import { useEffect, useRef } from 'react';
import type { Phase } from '../voice/machine';

interface Props {
  phase: Phase;
  /** Devuelve niveles 0–1 de entrada (usuario) y salida (JARVIS). */
  levels: () => { input: number; output: number };
  compact: boolean;
}

const TICKS = Array.from({ length: 60 }, (_, i) => i * 6);
const SEGMENTS = Array.from({ length: 10 }, (_, i) => i * 36);

/**
 * Núcleo animado de JARVIS. Los anillos giran según el estado y el núcleo
 * late con el nivel de audio (tu voz en verde-dorado, la de JARVIS en ámbar).
 * El nivel se aplica como variable CSS en cada fotograma, sin re-renderizar.
 */
export function Reactor({ phase, levels, compact }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let raf = 0;
    let smooth = 0;
    const tick = () => {
      const { input, output } = levels();
      const target = Math.max(input, output);
      smooth += (target - smooth) * 0.25;
      ref.current?.style.setProperty('--lvl', smooth.toFixed(3));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [levels]);

  return (
    <div ref={ref} className={`reactor phase-${phase} ${compact ? 'compact' : ''}`} aria-hidden="true">
      <svg viewBox="-320 -320 640 640">
        <defs>
          <radialGradient id="core" r="0.5">
            <stop offset="0%" stopColor="#fffaf0" />
            <stop offset="35%" stopColor="#f4e2b8" />
            <stop offset="70%" stopColor="#b8945a" stopOpacity="0.55" />
            <stop offset="100%" stopColor="#b8945a" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="halo" r="0.5">
            <stop offset="0%" stopColor="#f4e2b8" stopOpacity="0.35" />
            <stop offset="100%" stopColor="#f4e2b8" stopOpacity="0" />
          </radialGradient>
        </defs>

        <circle className="halo" r="300" fill="url(#halo)" />

        <g className="ring r-outer">
          <circle r="300" />
          <path d="M -300 0 A 300 300 0 0 1 212 -212" className="accent" />
        </g>
        <g className="ring r-dash">
          <circle r="250" strokeDasharray="2 10" />
        </g>
        <g className="ring r-mid">
          <circle r="200" />
          <path d="M 0 200 A 200 200 0 0 1 -173 -100" className="accent" />
        </g>
        <g className="ring r-ticks">
          {TICKS.map((a) => (
            <line key={a} x1="0" y1="-165" x2="0" y2={a % 30 === 0 ? -178 : -171} transform={`rotate(${a})`} />
          ))}
        </g>
        <g className="ring r-segments">
          <circle r="130" />
          {SEGMENTS.map((a) => (
            <path key={a} d="M 0 -122 L 0 -100" transform={`rotate(${a})`} />
          ))}
          <circle r="100" />
        </g>

        <g className="core">
          <circle r="90" fill="url(#core)" className="core-glow" />
          <circle r="34" className="core-solid" />
        </g>
      </svg>
    </div>
  );
}
