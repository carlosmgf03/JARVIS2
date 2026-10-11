import { useEffect, useState } from 'react';
import type { ApprovalRequest } from '../lib/types';

const RISK_LABEL = {
  read: 'Lectura',
  write: 'Escritura en la Vault',
  delegate: 'Envío a proveedor externo',
  web: 'Consulta a Internet',
  open_url: 'Abrir en el navegador',
  email: 'Lectura de correo',
};

interface Props {
  requests: ApprovalRequest[];
  onDecide: (id: string, approved: boolean) => void;
}

/** Confirmación explícita de operaciones de riesgo (voz o texto). */
export function ApprovalDialog({ requests, onDecide }: Props) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!requests.length) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [requests.length]);

  const req = requests[0];
  if (!req) return null;
  const remaining = Math.max(0, Math.round((req.expires_at_ms - now) / 1000));

  return (
    <div className="overlay" role="dialog" aria-modal="true" aria-labelledby="approval-title">
      <div className="dialog">
        <p className="eyebrow">
          Confirmación requerida · {RISK_LABEL[req.risk]} · origen: {req.origin === 'voice' ? 'voz' : 'texto'}
        </p>
        <h2 id="approval-title">{req.summary}</h2>
        {req.preview && <pre className="preview">{req.preview}</pre>}
        <p className="muted">
          JARVIS no hará nada hasta que decidas. Caduca en {remaining} s.
          {requests.length > 1 && ` (${requests.length - 1} más en cola)`}
        </p>
        <div className="row end">
          <button className="secondary" onClick={() => onDecide(req.id, false)}>
            Cancelar
          </button>
          <button className="primary" autoFocus onClick={() => onDecide(req.id, true)}>
            Autorizar
          </button>
        </div>
      </div>
    </div>
  );
}
