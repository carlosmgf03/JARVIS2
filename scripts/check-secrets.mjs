#!/usr/bin/env node
// Comprueba que no hay claves API en el repositorio ni en el frontend compilado.
//   node scripts/check-secrets.mjs          → ficheros versionados + dist/ si existe
// Sale con código 1 si encuentra algo con forma de clave.
import { execSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const PATTERNS = [
  { name: 'OpenAI API key', re: /sk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}/ },
  { name: 'Anthropic API key', re: /sk-ant-[A-Za-z0-9_-]{20,}/ },
  { name: 'OpenAI ephemeral key', re: /\bek_[A-Za-z0-9]{20,}/ },
];
// Ningún identificador de variable de clave debe acabar en el bundle del frontend.
const FRONTEND_FORBIDDEN = [/OPENAI_API_KEY/, /ANTHROPIC_API_KEY/];

const findings = [];

function scan(file, text, frontend) {
  for (const { name, re } of PATTERNS) if (re.test(text)) findings.push(`${file}: ${name}`);
  if (frontend) for (const re of FRONTEND_FORBIDDEN) if (re.test(text)) findings.push(`${file}: referencia a ${re.source} en el frontend`);
}

const tracked = execSync('git ls-files -co --exclude-standard', { encoding: 'utf8' })
  .split('\n')
  .filter(Boolean)
  .filter((f) => !/\.(png|ico|icns|lock)$/.test(f) && f !== 'package-lock.json' && existsSync(f));
for (const f of tracked) {
  if (statSync(f).size > 2_000_000) continue;
  const text = readFileSync(f, 'utf8');
  scan(f, text, f.startsWith('src/') && !f.endsWith('.test.ts'));
}

function walk(dir) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p);
    else if (/\.(js|html|css|map|json)$/.test(e.name)) scan(p, readFileSync(p, 'utf8'), true);
  }
}
if (existsSync('dist')) walk('dist');

if (findings.length) {
  console.error('Posibles secretos encontrados:\n' + findings.map((f) => `  - ${f}`).join('\n'));
  process.exit(1);
}
console.log(`Sin secretos: ${tracked.length} ficheros versionados${existsSync('dist') ? ' + dist/' : ''} revisados.`);
