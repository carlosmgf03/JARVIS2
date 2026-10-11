// E/S de audio PCM16 a 24 kHz para el transporte WebSocket de respaldo.
//
// Con WebRTC (transporte principal) el navegador gestiona captura,
// reproducción y cancelación de eco. Algunos WebView (p. ej. WebKitGTK en
// ciertas distribuciones Linux) no incluyen WebRTC; en ese caso JARVIS usa el
// transporte WebSocket oficial del SDK y esta clase captura el micrófono y
// reproduce el audio recibido.

import { analyserLevel } from './levels';

export const SAMPLE_RATE = 24_000;
const CHUNK_SAMPLES = 2_400; // 100 ms

export function floatToPcm16(input: Float32Array): Int16Array {
  const out = new Int16Array(input.length);
  for (let i = 0; i < input.length; i++) {
    const s = Math.max(-1, Math.min(1, input[i]));
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
}

export function pcm16ToFloat(buf: ArrayBuffer): Float32Array {
  const pcm = new Int16Array(buf);
  const out = new Float32Array(pcm.length);
  for (let i = 0; i < pcm.length; i++) out[i] = pcm[i] / 0x8000;
  return out;
}

export class PcmAudioIO {
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private gain: GainNode | null = null;
  private analyser: AnalyserNode | null = null;
  private levelBuf = new Uint8Array(512);
  private pending: Float32Array[] = [];
  private pendingLen = 0;
  private playing = new Set<AudioBufferSourceNode>();
  private nextTime = 0;
  captureEnabled = true;

  constructor(
    private readonly stream: MediaStream,
    private readonly onChunk: (pcm: ArrayBuffer) => void,
    private readonly onDrained: () => void,
  ) {}

  async start(): Promise<void> {
    this.ctx = new AudioContext({ sampleRate: SAMPLE_RATE });
    await this.ctx.audioWorklet.addModule('/pcm-capture-worklet.js');
    this.source = this.ctx.createMediaStreamSource(this.stream);
    this.node = new AudioWorkletNode(this.ctx, 'pcm-capture');
    this.node.port.onmessage = (e: MessageEvent<Float32Array>) => this.capture(e.data);
    this.source.connect(this.node);
    this.gain = this.ctx.createGain();
    this.gain.connect(this.ctx.destination);
    this.analyser = this.ctx.createAnalyser();
    this.analyser.fftSize = 512;
    this.gain.connect(this.analyser);
    if (this.ctx.state === 'suspended') await this.ctx.resume();
  }

  private capture(frame: Float32Array) {
    if (!this.captureEnabled) return;
    this.pending.push(frame);
    this.pendingLen += frame.length;
    if (this.pendingLen < CHUNK_SAMPLES) return;
    const merged = new Float32Array(this.pendingLen);
    let off = 0;
    for (const f of this.pending) {
      merged.set(f, off);
      off += f.length;
    }
    this.pending = [];
    this.pendingLen = 0;
    this.onChunk(floatToPcm16(merged).buffer as ArrayBuffer);
  }

  play(pcm: ArrayBuffer): void {
    if (!this.ctx || !this.gain) return;
    const data = pcm16ToFloat(pcm);
    if (!data.length) return;
    const buffer = this.ctx.createBuffer(1, data.length, SAMPLE_RATE);
    buffer.copyToChannel(data as Float32Array<ArrayBuffer>, 0);
    const src = this.ctx.createBufferSource();
    src.buffer = buffer;
    src.connect(this.gain);
    const start = Math.max(this.ctx.currentTime + 0.02, this.nextTime);
    src.start(start);
    this.nextTime = start + buffer.duration;
    this.playing.add(src);
    src.onended = () => {
      this.playing.delete(src);
      if (this.playing.size === 0) this.onDrained();
    };
  }

  /** Corta en seco la reproducción (interrupción). */
  stopPlayback(): void {
    for (const s of this.playing) {
      s.onended = null;
      try {
        s.stop();
      } catch {
        /* ya detenido */
      }
    }
    this.playing.clear();
    this.nextTime = 0;
  }

  /** Nivel 0–1 del audio que se está reproduciendo (para la animación). */
  outputLevel(): number {
    return analyserLevel(this.analyser, this.levelBuf);
  }

  setVolume(v: number): void {
    if (this.gain) this.gain.gain.value = v;
  }

  async close(): Promise<void> {
    this.stopPlayback();
    this.source?.disconnect();
    this.node?.disconnect();
    if (this.node) this.node.port.onmessage = null;
    this.gain?.disconnect();
    await this.ctx?.close().catch(() => undefined);
    this.ctx = null;
  }
}
