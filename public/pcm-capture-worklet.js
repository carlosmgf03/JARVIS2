// AudioWorklet de captura para el transporte WebSocket de respaldo.
// Envía al hilo principal bloques Float32 del canal 0 (el contexto se crea a
// 24 kHz, de modo que el navegador ya remuestrea el micrófono).
class PcmCapture extends AudioWorkletProcessor {
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel && channel.length) this.port.postMessage(channel.slice(0));
    return true;
  }
}
registerProcessor('pcm-capture', PcmCapture);
