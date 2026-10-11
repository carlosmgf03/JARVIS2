import { useEffect, useRef, useState } from 'react';
import { micError } from '../voice/controller';

/**
 * Prueba local del micrófono: mide el nivel de entrada sin enviar audio a
 * ningún sitio. Sirve para comprobar permisos y hardware sin credenciales.
 */
export function MicTest() {
  const [level, setLevel] = useState(0);
  const [active, setActive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cleanup = useRef<() => void>(() => {});

  useEffect(() => () => cleanup.current(), []);

  async function start() {
    setError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true } });
      const ctx = new AudioContext();
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      ctx.createMediaStreamSource(stream).connect(analyser);
      const data = new Uint8Array(analyser.fftSize);
      let raf = 0;
      const tick = () => {
        analyser.getByteTimeDomainData(data);
        let peak = 0;
        for (const v of data) peak = Math.max(peak, Math.abs(v - 128));
        setLevel(peak / 128);
        raf = requestAnimationFrame(tick);
      };
      tick();
      cleanup.current = () => {
        cancelAnimationFrame(raf);
        stream.getTracks().forEach((t) => t.stop());
        void ctx.close();
        setActive(false);
        setLevel(0);
      };
      setActive(true);
    } catch (e) {
      setError(micError(e).message);
    }
  }

  return (
    <div className="mictest">
      <div className="row">
        {active ? (
          <button className="secondary" onClick={() => cleanup.current()}>
            Apagar micrófono
          </button>
        ) : (
          <button className="secondary" onClick={start}>
            Probar micrófono (local)
          </button>
        )}
        <div className="meter" aria-label="Nivel del micrófono">
          <div style={{ width: `${Math.round(level * 100)}%` }} />
        </div>
      </div>
      {error && <p className="alert error">{error}</p>}
    </div>
  );
}
