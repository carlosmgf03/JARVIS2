#!/bin/bash
# Instala JARVIS como aplicación normal en /Applications (macOS).
# Uso: npm run app:install   (o: bash scripts/instalar-mac.sh)
#
# - Guarda la configuración (sin claves) en
#   ~/Library/Application Support/com.jarvis.desktop/config.toml
# - Compila la app y la copia a /Applications/JARVIS.app
# Las claves siguen en el Llavero (jarvis secrets set …), nunca en este fichero.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname)" != "Darwin" ]]; then
  echo "Este instalador es para macOS." >&2
  exit 1
fi
command -v npm >/dev/null || { echo "Falta Node.js (https://nodejs.org)." >&2; exit 1; }
command -v cargo >/dev/null || { echo "Falta Rust (https://rustup.rs)." >&2; exit 1; }

CONFIG_DIR="$HOME/Library/Application Support/com.jarvis.desktop"
CONFIG="$CONFIG_DIR/config.toml"
mkdir -p "$CONFIG_DIR"

clean_path() {
  local p="$1"
  p="${p#"${p%%[![:space:]]*}"}"; p="${p%"${p##*[![:space:]]}"}"   # recorta espacios
  p="${p#\'}"; p="${p%\'}"; p="${p#\"}"; p="${p%\"}"                # quita comillas
  p="${p//\\ / }"                                                    # «\ » -> « »
  printf '%s' "$p"
}
toml() { local v="${1//\\/\\\\}"; v="${v//\"/\\\"}"; printf '"%s"' "$v"; }

if [[ ! -f "$CONFIG" ]]; then
  echo "== Configuración inicial de JARVIS =="
  echo "(Pulsa Enter para dejar un campo vacío. Podrás cambiarlo en: $CONFIG)"
  echo
  read -r -p "1) Arrastra aquí la carpeta de tus notas de Obsidian y pulsa Enter: " VAULT
  VAULT="$(clean_path "$VAULT")"
  read -r -p "2) Ciudad para el tiempo [Bogotá]: " CITY
  read -r -p "3) Dirección de tu casa, para el tráfico (p. ej. Calle 100 # 15-20, Bogotá): " HOME_ADDR
  read -r -p "4) Dirección de tu trabajo: " WORK_ADDR
  read -r -p "5) Tu correo (solo si configuraste la contraseña de aplicación): " EMAIL
  {
    echo "# Configuración de JARVIS (sin claves: las claves van en el Llavero)."
    [[ -n "$VAULT" ]] && echo "JARVIS_VAULT_PATH = $(toml "$VAULT")"
    echo "JARVIS_WEATHER_LOCATION = $(toml "${CITY:-Bogotá}")"
    [[ -n "$HOME_ADDR" ]] && echo "JARVIS_HOME_ADDRESS = $(toml "$HOME_ADDR")"
    [[ -n "$WORK_ADDR" ]] && echo "JARVIS_WORK_ADDRESS = $(toml "$WORK_ADDR")"
    [[ -n "$EMAIL" ]] && echo "JARVIS_EMAIL_ADDRESS = $(toml "$EMAIL")"
    true
  } > "$CONFIG"
  echo "Configuración guardada en: $CONFIG"
else
  echo "Usando la configuración existente: $CONFIG"
fi

[[ -d node_modules ]] || npm ci
echo "== Compilando JARVIS (la primera vez tarda varios minutos) =="
npm run tauri -- build --bundles app

APP="target/release/bundle/macos/JARVIS.app"
[[ -d "$APP" ]] || { echo "No se encontró $APP" >&2; exit 1; }
rm -rf "/Applications/JARVIS.app"
cp -R "$APP" /Applications/
echo
echo "✔ JARVIS instalado en Aplicaciones."
echo "  Ábrelo con Cmd+Espacio → «JARVIS», desde el Dock o diciendo «Oye Siri, abre JARVIS»."
open /Applications/JARVIS.app
