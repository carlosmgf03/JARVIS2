//! Búsqueda de vuelos con datos de Google Flights a través de SerpApi.
//!
//! Para la fecha pedida devuelve las mejores opciones (precio, aerolínea,
//! horarios, duración y escalas) y, si se piden días flexibles, busca también
//! los días cercanos para indicar cuál es el más económico.
//!
//! Cada fecha consultada cuenta como una búsqueda de SerpApi (plan gratuito:
//! 250 al mes). Los precios son orientativos y deben confirmarse antes de
//! comprar. Requiere `jarvis secrets set serpapi`.

use std::time::Duration;

use chrono::{Days, NaiveDate};
use serde::Serialize;
use serde_json::Value;

use crate::config::FlightsConfig;
use crate::error::{ErrorCode, JarvisError, Result};
use crate::http;
use crate::secrets::Secret;

#[derive(Debug, Clone)]
pub struct FlightQuery {
    pub origin: String,
    pub destination: String,
    pub date: NaiveDate,
    pub return_date: Option<NaiveDate>,
    pub flexible_days: u32,
    pub adults: u32,
    pub cabin: String,
    pub nonstop: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlightOption {
    pub date: String,
    pub price: f64,
    pub airlines: String,
    pub flight_numbers: String,
    pub departure: String,
    pub arrival: String,
    pub duration_minutes: Option<i64>,
    pub stops: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayCheapest {
    pub date: String,
    pub cheapest: Option<FlightOption>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlightResults {
    pub route: String,
    pub trip: &'static str,
    pub currency: String,
    pub requested_date: String,
    /// Mejores opciones de la fecha pedida, de más barata a más cara.
    pub options: Vec<FlightOption>,
    /// El más económico de cada día consultado.
    pub by_date: Vec<DayCheapest>,
    /// El más económico de todos los días consultados.
    pub cheapest_overall: Option<FlightOption>,
    pub price_insights: Option<Value>,
    pub searches_used: usize,
    pub source: &'static str,
    pub note: &'static str,
}

/// Valida códigos IATA (uno o varios separados por comas): `BOG`, `MDE,EOH`.
pub fn normalize_airports(s: &str) -> Result<String> {
    let codes: Vec<String> = s
        .split(',')
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty())
        .collect();
    if codes.is_empty()
        || codes.len() > 4
        || !codes
            .iter()
            .all(|c| c.len() == 3 && c.chars().all(|ch| ch.is_ascii_uppercase()))
    {
        return Err(JarvisError::invalid(
            "Usa códigos de aeropuerto IATA de 3 letras (p. ej. BOG, MDE, MIA).",
        ));
    }
    Ok(codes.join(","))
}

fn cabin_code(c: &str) -> &'static str {
    match c {
        "premium" => "2",
        "business" => "3",
        "first" => "4",
        _ => "1",
    }
}

/// Fechas a consultar: la pedida y ±`flex` días, sin fechas pasadas.
pub fn dates_to_search(date: NaiveDate, flex: u32, today: NaiveDate) -> Vec<NaiveDate> {
    let mut out = vec![date];
    for d in 1..=flex {
        if let Some(before) = date.checked_sub_days(Days::new(d.into())) {
            if before >= today {
                out.push(before);
            }
        }
        if let Some(after) = date.checked_add_days(Days::new(d.into())) {
            out.push(after);
        }
    }
    out
}

/// Convierte una respuesta de `engine=google_flights` en opciones.
pub fn parse_options(v: &Value, date: &str) -> Vec<FlightOption> {
    let mut out: Vec<FlightOption> = ["best_flights", "other_flights"]
        .iter()
        .flat_map(|k| v[*k].as_array().cloned().unwrap_or_default())
        .filter_map(|o| {
            let price = o["price"].as_f64()?;
            let legs = o["flights"].as_array()?;
            let first = legs.first()?;
            let last = legs.last()?;
            let mut airlines: Vec<&str> =
                legs.iter().filter_map(|l| l["airline"].as_str()).collect();
            airlines.dedup();
            Some(FlightOption {
                date: date.to_string(),
                price,
                airlines: airlines.join(" + "),
                flight_numbers: legs
                    .iter()
                    .filter_map(|l| l["flight_number"].as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                departure: format!(
                    "{} {}",
                    first["departure_airport"]["id"].as_str().unwrap_or(""),
                    first["departure_airport"]["time"].as_str().unwrap_or("")
                ),
                arrival: format!(
                    "{} {}",
                    last["arrival_airport"]["id"].as_str().unwrap_or(""),
                    last["arrival_airport"]["time"].as_str().unwrap_or("")
                ),
                duration_minutes: o["total_duration"].as_i64(),
                stops: legs.len().saturating_sub(1),
            })
        })
        .collect();
    out.sort_by(|a, b| a.price.total_cmp(&b.price));
    out
}

pub struct FlightsClient {
    http: reqwest::Client,
    cfg: FlightsConfig,
}

impl FlightsClient {
    pub fn new(cfg: FlightsConfig) -> Self {
        Self {
            http: http::client(),
            cfg,
        }
    }

    pub fn home_airport(&self) -> &str {
        &self.cfg.home_airport
    }

    async fn search_day(&self, key: &Secret, q: &FlightQuery, date: NaiveDate) -> Result<Value> {
        let date_s = date.format("%Y-%m-%d").to_string();
        let mut params: Vec<(&str, String)> = vec![
            ("engine", "google_flights".into()),
            ("departure_id", q.origin.clone()),
            ("arrival_id", q.destination.clone()),
            ("outbound_date", date_s),
            (
                "type",
                if q.return_date.is_some() { "1" } else { "2" }.into(),
            ),
            ("currency", self.cfg.currency.clone()),
            ("hl", "es".into()),
            ("gl", "co".into()),
            ("adults", q.adults.to_string()),
            ("travel_class", cabin_code(&q.cabin).into()),
        ];
        if let Some(r) = q.return_date {
            // Mantiene la duración del viaje al mover la fecha de ida.
            let shift = date.signed_duration_since(q.date);
            params.push(("return_date", (r + shift).format("%Y-%m-%d").to_string()));
        }
        if q.nonstop {
            params.push(("stops", "1".into()));
        }
        params.push(("api_key", key.expose().to_string()));
        let resp = self
            .http
            .get(format!(
                "{}/search.json",
                self.cfg.serpapi_base_url.trim_end_matches('/')
            ))
            .query(&params)
            .timeout(Duration::from_secs(40))
            .send()
            .await
            .map_err(|e| http::network_error("SerpApi", e))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| http::network_error("SerpApi", e))?;
        if !(200..300).contains(&status) {
            return Err(http::status_error("SerpApi", status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| {
            JarvisError::new(ErrorCode::ProviderError, "Respuesta inesperada de SerpApi.")
        })?;
        if let Some(e) = v["error"].as_str() {
            // «No results» no es un error: simplemente no hay vuelos ese día.
            if !e.to_lowercase().contains("no results")
                && !e.to_lowercase().contains("hasn't returned any results")
            {
                return Err(JarvisError::new(
                    ErrorCode::ProviderError,
                    format!("SerpApi: {}", crate::secrets::redact(e)),
                ));
            }
        }
        Ok(v)
    }

    pub async fn search(
        &self,
        key: &Secret,
        q: &FlightQuery,
        today: NaiveDate,
    ) -> Result<FlightResults> {
        let dates = dates_to_search(q.date, q.flexible_days, today);
        let responses =
            futures::future::join_all(dates.iter().map(|d| self.search_day(key, q, *d))).await;
        let mut by_date = Vec::new();
        let mut options = Vec::new();
        let mut insights = None;
        let mut first_error = None;
        for (d, r) in dates.iter().zip(responses) {
            let ds = d.format("%Y-%m-%d").to_string();
            match r {
                Ok(v) => {
                    let opts = parse_options(&v, &ds);
                    if *d == q.date {
                        insights = v.get("price_insights").cloned();
                        options = opts.iter().take(5).cloned().collect();
                    }
                    by_date.push(DayCheapest {
                        date: ds,
                        cheapest: opts.into_iter().next(),
                    });
                }
                Err(e) if *d == q.date => return Err(e),
                Err(e) => {
                    first_error.get_or_insert(e);
                    by_date.push(DayCheapest {
                        date: ds,
                        cheapest: None,
                    });
                }
            }
        }
        by_date.sort_by(|a, b| a.date.cmp(&b.date));
        let cheapest_overall = by_date
            .iter()
            .filter_map(|d| d.cheapest.clone())
            .min_by(|a, b| a.price.total_cmp(&b.price));
        Ok(FlightResults {
            route: format!("{} → {}", q.origin, q.destination),
            trip: if q.return_date.is_some() { "ida y vuelta (precio total)" } else { "solo ida" },
            currency: self.cfg.currency.clone(),
            requested_date: q.date.format("%Y-%m-%d").to_string(),
            options,
            by_date,
            cheapest_overall,
            price_insights: insights,
            searches_used: dates.len(),
            source: "Google Flights (vía SerpApi)",
            note: "Precios orientativos en el momento de la consulta; confírmalos en la aerolínea o agencia antes de comprar.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn airports_and_dates() {
        assert_eq!(normalize_airports("bog").unwrap(), "BOG");
        assert_eq!(normalize_airports("MDE, eoh").unwrap(), "MDE,EOH");
        assert!(normalize_airports("Bogotá").is_err());
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let ds = dates_to_search(d("2026-10-12"), 2, d("2026-10-11"));
        assert_eq!(
            ds,
            vec![
                d("2026-10-12"),
                d("2026-10-11"),
                d("2026-10-13"),
                d("2026-10-14")
            ]
        );
    }

    #[test]
    fn parses_google_flights_results() {
        let v = json!({
            "best_flights": [{"price": 320000, "total_duration": 60, "flights": [
                {"airline": "Avianca", "flight_number": "AV 9312",
                 "departure_airport": {"id": "BOG", "time": "2026-10-20 06:00"},
                 "arrival_airport": {"id": "MDE", "time": "2026-10-20 07:00"}}]}],
            "other_flights": [{"price": 250000, "total_duration": 180, "flights": [
                {"airline": "LATAM", "flight_number": "LA 4040",
                 "departure_airport": {"id": "BOG", "time": "2026-10-20 05:00"},
                 "arrival_airport": {"id": "CLO", "time": "2026-10-20 06:00"}},
                {"airline": "LATAM", "flight_number": "LA 4041",
                 "departure_airport": {"id": "CLO", "time": "2026-10-20 07:00"},
                 "arrival_airport": {"id": "MDE", "time": "2026-10-20 08:00"}}]}]
        });
        let o = parse_options(&v, "2026-10-20");
        assert_eq!(o[0].price, 250000.0);
        assert_eq!(o[0].stops, 1);
        assert_eq!(o[0].airlines, "LATAM");
        assert_eq!(o[1].departure, "BOG 2026-10-20 06:00");
    }
}
