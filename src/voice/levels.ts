// Medidor de nivel de audio para animar la interfaz (núcleo del reactor).
// Solo lee niveles en el dispositivo: no graba ni envía audio a ningún sitio.

export function rms(data: Uint8Array): number {
  let sum = 0;
  for (const v of data) {
    const x = (v - 128) / 128;
    sum += x * x;
  }
  return data.length ? Math.sqrt(sum / data.length) : 0;
}

/** Nivel 0–1 de un analizador, con algo de ganancia para que la voz se note. */
export function analyserLevel(a: AnalyserNode | null, buf: Uint8Array<ArrayBuffer> | null): number {
  if (!a || !buf) return 0;
  a.getByteTimeDomainData(buf);
  return Math.min(1, rms(buf) * 4);
}

export class LevelMeter {
  private ctx: AudioContext | null = null;
  private input: AnalyserNode | null = null;
  private output: AnalyserNode | null = null;
  private outputStream: MediaStream | null = null;
  private buf: Uint8Array<ArrayBuffer> | null = null;

  private context(): AudioContext | null {
    if (typeof AudioContext === 'undefined') return null;
    if (!this.ctx) {
      try {
        this.ctx = new AudioContext();
        this.buf = new Uint8Array(512);
      } catch {
        return null;
      }
    }
    return this.ctx;
  }

  private analyser(stream: MediaStream): AnalyserNode | null {
    const ctx = this.context();
    if (!ctx) return null;
    try {
      const a = ctx.createAnalyser();
      a.fftSize = 512;
      a.smoothingTimeConstant = 0.6;
      ctx.createMediaStreamSource(stream).connect(a);
      if (ctx.state === 'suspended') void ctx.resume();
      return a;
    } catch {
      return null;
    }
  }

  attachInput(stream: MediaStream): void {
    this.input = this.analyser(stream);
  }

  /** Audio remoto de WebRTC (se conecta en cuanto el SDK asigna la pista). */
  attachOutput(stream: MediaStream): void {
    if (this.outputStream === stream || !stream.getAudioTracks().length) return;
    this.outputStream = stream;
    this.output = this.analyser(stream);
  }

  inputLevel(): number {
    return analyserLevel(this.input, this.buf);
  }

  outputLevel(): number {
    return analyserLevel(this.output, this.buf);
  }

  close(): void {
    this.input = null;
    this.output = null;
    this.outputStream = null;
    void this.ctx?.close().catch(() => undefined);
    this.ctx = null;
  }
}
