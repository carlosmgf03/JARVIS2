import { useEffect, useRef, type ReactNode } from 'react';

/** Lista de mensajes que se desplaza sola al último elemento. */
export function Messages({ children, count }: { children: ReactNode; count: number }) {
  const end = useRef<HTMLLIElement>(null);
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: 'nearest' });
  }, [count]);
  return (
    <ol className="messages">
      {children}
      <li ref={end} aria-hidden="true" className="sentinel" />
    </ol>
  );
}
