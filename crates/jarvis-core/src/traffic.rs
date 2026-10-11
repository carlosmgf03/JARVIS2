//! Tráfico en vivo (TomTom), centrado por defecto en Bogotá.
//!
//! - **Ruta**: tiempo de viaje con tráfico actual, retraso frente a la vía
//!   libre, distancia y hora estimada de llegada. Admite «casa» y «trabajo»
//!   como alias de direcciones configuradas.
//! - **Incidentes**: accidentes, cierres, obras y trancones en el área (por
//!   defecto, el rectángulo de Bogotá), ordenados por gravedad.
//!
//! Requiere una clave gratuita de TomTom (`jarvis secrets set tomtom`). Sin
//! clave, la herramienta no se ofrece y JARVIS usa la búsqueda web con fuentes
//! de Bogotá.

use std::time::Duration;

use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

use crate::config::TrafficConfig;
use crate::error::{ErrorCode, JarvisError, Result};
use crate::http;
use crate::secrets::Secret;

#[derive(Debug, Clone, Serialize)]
pub struct RouteSummary {
    pub from: String,
    pub to: String,
    pub minutes_with_traffic: f64,
    pub minutes_without_traffic: Option<f64>,
    pub delay_minutes: f64,
    pub distance_km: f64,
    pub arrival_time: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Incident {
    pub kind: &'static str,
    pub description: String,
    pub road: String,
    pub delay_minutes: Option<f64>,
    pub severity: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrafficReport {
    pub area: String,
    pub route: Option<RouteSummary>,
    pub incidents: Vec<Incident>,
    pub source: &'static str,
}

/// Categoría de incidente de TomTom (`iconCategory`) en español.
pub fn incident_kind(code: i64) -> &'static str {
    match code {
        1 => "accidente",
        2 => "niebla",
        3 => "condiciones peligrosas",
        4 => "lluvia",
        5 => "hielo",
        6 => "trancón",
        7 => "carril cerrado",
        8 => "vía cerrada",
        9 => "obras",
        10 => "viento",
        11 => "inundación",
        14 => "vehículo varado",
        _ => "incidente",
    }
}

/// Gravedad del retraso (`magnitudeOfDelay`).
pub fn severity(code: i64) -> &'static str {
    match code {
        1 => "leve",
        2 => "moderado",
        3 => "grave",
        4 => "cierre",
        _ => "desconocido",
    }
}

pub struct TrafficClient {
    http: reqwest::Client,
    cfg: TrafficConfig,
}

impl TrafficClient {
    pub fn new(cfg: TrafficConfig) -> Self {
        Self {
            http: http::client(),
            cfg,
        }
    }

    pub fn area(&self) -> &str {
        &self.cfg.city
    }

    /// Traduce «casa»/«trabajo» a la dirección configurada.
    fn resolve_alias(&self, place: &str) -> String {
        let p = crate::vault::normalize(place.trim());
        let alias = match p.as_str() {
            "casa" | "mi casa" | "hogar" => self.cfg.home_address.clone(),
            "trabajo" | "mi trabajo" | "oficina" | "la oficina" => self.cfg.work_address.clone(),
            _ => None,
        };
        alias.unwrap_or_else(|| place.trim().to_string())
    }

    pub async fn report(
        &self,
        key: &Secret,
        origin: Option<&str>,
        destination: Option<&str>,
    ) -> Result<TrafficReport> {
        let route = match (origin, destination) {
            (Some(o), Some(d)) => Some(self.route(key, o, d).await?),
            (None, Some(d)) => match self.cfg.home_address.as_deref() {
                Some(_) => Some(self.route(key, "casa", d).await?),
                None => {
                    return Err(JarvisError::invalid(
                        "Falta el punto de partida. Pregunta al usuario desde dónde sale o configura JARVIS_HOME_ADDRESS.",
                    ))
                }
            },
            _ => None,
        };
        let incidents = self.incidents(key).await?;
        Ok(TrafficReport {
            area: self.cfg.city.clone(),
            route,
            incidents,
            source: "TomTom",
        })
    }

    async fn get(&self, url: Url) -> Result<Value> {
        let resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| http::network_error("TomTom", e))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| http::network_error("TomTom", e))?;
        if !(200..300).contains(&status) {
            return Err(http::status_error("TomTom", status, &text));
        }
        serde_json::from_str(&text).map_err(|_| {
            JarvisError::new(ErrorCode::ProviderError, "Respuesta inesperada de TomTom.")
        })
    }

    fn url(&self, path: &[&str]) -> Result<Url> {
        let mut url = Url::parse(&self.cfg.tomtom_base_url)
            .map_err(|_| JarvisError::new(ErrorCode::Config, "URL de TomTom no válida."))?;
        url.path_segments_mut()
            .map_err(|_| JarvisError::new(ErrorCode::Config, "URL de TomTom no válida."))?
            .pop_if_empty()
            .extend(path);
        Ok(url)
    }

    /// Geocodifica una dirección o lugar, priorizando Colombia y la ciudad.
    async fn geocode(&self, key: &Secret, place: &str) -> Result<(f64, f64, String)> {
        let q = self.resolve_alias(place);
        let mut url = self.url(&["search", "2", "geocode", &format!("{q}.json")])?;
        url.query_pairs_mut()
            .append_pair("key", key.expose())
            .append_pair("countrySet", &self.cfg.country)
            .append_pair("lat", &self.cfg.center_lat.to_string())
            .append_pair("lon", &self.cfg.center_lon.to_string())
            .append_pair("limit", "1")
            .append_pair("language", "es-ES");
        let v = self.get(url).await?;
        let r = v["results"].get(0).ok_or_else(|| {
            JarvisError::new(
                ErrorCode::NotFound,
                format!("No encontré la dirección «{q}»."),
            )
        })?;
        let lat = r["position"]["lat"].as_f64().unwrap_or_default();
        let lon = r["position"]["lon"].as_f64().unwrap_or_default();
        let name = r["address"]["freeformAddress"]
            .as_str()
            .unwrap_or(&q)
            .to_string();
        Ok((lat, lon, name))
    }

    async fn route(&self, key: &Secret, origin: &str, destination: &str) -> Result<RouteSummary> {
        let (a, b) = tokio::try_join!(self.geocode(key, origin), self.geocode(key, destination))?;
        let locs = format!("{},{}:{},{}", a.0, a.1, b.0, b.1);
        let mut url = self.url(&["routing", "1", "calculateRoute", &locs, "json"])?;
        url.query_pairs_mut()
            .append_pair("key", key.expose())
            .append_pair("traffic", "true")
            .append_pair("travelMode", "car")
            .append_pair("departAt", "now")
            .append_pair("computeTravelTimeFor", "all");
        let v = self.get(url).await?;
        let s = &v["routes"][0]["summary"];
        let secs = s["travelTimeInSeconds"].as_f64().ok_or_else(|| {
            JarvisError::new(
                ErrorCode::NotFound,
                "TomTom no encontró una ruta entre esos puntos.",
            )
        })?;
        Ok(RouteSummary {
            from: a.2,
            to: b.2,
            minutes_with_traffic: (secs / 60.0).round(),
            minutes_without_traffic: s["noTrafficTravelTimeInSeconds"]
                .as_f64()
                .map(|x| (x / 60.0).round()),
            delay_minutes: (s["trafficDelayInSeconds"].as_f64().unwrap_or(0.0) / 60.0).round(),
            distance_km: (s["lengthInMeters"].as_f64().unwrap_or(0.0) / 100.0).round() / 10.0,
            arrival_time: s["arrivalTime"].as_str().map(str::to_string),
        })
    }

    async fn incidents(&self, key: &Secret) -> Result<Vec<Incident>> {
        let mut url = self.url(&["traffic", "services", "5", "incidentDetails"])?;
        url.query_pairs_mut()
            .append_pair("key", key.expose())
            .append_pair("bbox", &self.cfg.bbox)
            .append_pair(
                "fields",
                "{incidents{properties{iconCategory,magnitudeOfDelay,events{description},from,to,delay,roadNumbers}}}",
            )
            .append_pair("language", "es-ES")
            .append_pair("timeValidityFilter", "present");
        let v = self.get(url).await?;
        let mut list: Vec<(i64, f64, Incident)> = v["incidents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                let p = &i["properties"];
                let mag = p["magnitudeOfDelay"].as_i64().unwrap_or(0);
                let delay = p["delay"].as_f64();
                let desc = p["events"]
                    .as_array()
                    .and_then(|e| e.first())
                    .and_then(|e| e["description"].as_str())
                    .unwrap_or("")
                    .to_string();
                let road = [p["from"].as_str(), p["to"].as_str()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" → ");
                let road = match p["roadNumbers"]
                    .as_array()
                    .and_then(|r| r.first())
                    .and_then(Value::as_str)
                {
                    Some(n) if !road.is_empty() => format!("{n}: {road}"),
                    Some(n) => n.to_string(),
                    None => road,
                };
                (
                    mag,
                    delay.unwrap_or(0.0),
                    Incident {
                        kind: incident_kind(p["iconCategory"].as_i64().unwrap_or(0)),
                        description: desc,
                        road,
                        delay_minutes: delay.map(|d| (d / 60.0).round()),
                        severity: severity(mag),
                    },
                )
            })
            .collect();
        // Primero cierres y retrasos graves; después los de más minutos.
        list.sort_by(|a, b| {
            let rank = |m: i64| if m == 4 { 5 } else { m };
            rank(b.0).cmp(&rank(a.0)).then(b.1.total_cmp(&a.1))
        });
        Ok(list.into_iter().take(8).map(|(_, _, i)| i).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_labels() {
        assert_eq!(incident_kind(1), "accidente");
        assert_eq!(incident_kind(6), "trancón");
        assert_eq!(severity(3), "grave");
    }
}
