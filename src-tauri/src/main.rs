// Evita una consola adicional en Windows en compilaciones de producción.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    jarvis_desktop_lib::run()
}
