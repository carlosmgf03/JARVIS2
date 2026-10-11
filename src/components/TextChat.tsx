import { useState, type FormEvent } from 'react';
import type { Backend } from '../lib/backend';
import { toJarvisError } from '../lib/backend';
import { Messages } from './Messages';

interface Message {
  role: 'user' | 'assistant' | 'error';
  text: string;
  meta?: string;
}

interface Props {
  backend: Backend;
  /** Si hay sesión de voz activa, el texto se envía a Realtime. */
  sendToVoice: (text: string) => boolean;
  voiceContext: () => string | null;
  provider: 'claude' | 'commands';
  onExchange?: (user: string, assistant: string) => void;
}

export function TextChat({ backend, sendToVoice, voiceContext, provider, onExchange }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const text = input.trim();
    if (!text || busy) return;
    setInput('');
    if (!text.startsWith('/') && sendToVoice(text)) {
      setMessages((m) => [...m, { role: 'user', text, meta: 'enviado a la sesión de voz' }]);
      return;
    }
    setMessages((m) => [...m, { role: 'user', text }]);
    setBusy(true);
    try {
      const r = await backend.sendText(text, voiceContext());
      const tools = r.tools.map((t) => `${t.tool}: ${t.status}${t.code ? ` (${t.code})` : ''}`).join(', ');
      setMessages((m) => [...m, { role: 'assistant', text: r.reply, meta: [r.provider, tools].filter(Boolean).join(' · ') }]);
      onExchange?.(text, r.reply);
    } catch (err) {
      setMessages((m) => [...m, { role: 'error', text: toJarvisError(err).message }]);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card chat" aria-label="Interfaz de texto">
      <header className="row between">
        <h2>Texto</h2>
        <span className="tag">{provider === 'claude' ? 'Claude + herramientas' : 'modo comandos (/ayuda)'}</span>
      </header>
      <Messages count={messages.length + (busy ? 1 : 0)}>
        {messages.map((m, i) => (
          <li key={i} className={`msg ${m.role}`}>
            <pre>{m.text}</pre>
            {m.meta && <small>{m.meta}</small>}
          </li>
        ))}
        {busy && <li className="msg assistant muted">JARVIS está trabajando…</li>}
      </Messages>
      <form className="row" onSubmit={submit}>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={provider === 'claude' ? 'Escribe a JARVIS…' : 'Prueba /tareas hoy, /buscar …, /resumen'}
          aria-label="Mensaje"
        />
        <button className="primary" disabled={busy || !input.trim()}>
          Enviar
        </button>
      </form>
    </section>
  );
}
