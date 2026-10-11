import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Tauri espera un puerto fijo y no debe abrir el navegador.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Solo variables VITE_* llegan al frontend: OPENAI_API_KEY nunca se incrusta.
  envPrefix: ['VITE_'],
  build: { target: 'es2022', sourcemap: false },
  test: { environment: 'jsdom', include: ['src/**/*.test.ts', 'src/**/*.test.tsx'] },
});
