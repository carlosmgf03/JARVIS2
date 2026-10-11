//! Lugares del mundo: búsqueda de ciudades y países (geocodificación con
//! Open-Meteo) y hora local de cualquier sitio.
//!
//! - Si el usuario nombra un **país**, se usa su **capital** (el tiempo en el
//!   centro geográfico de un país no tiene sentido).
//! - La hora se calcula **en el equipo** con la base de datos de zonas
//!   horarias IANA (`chrono-tz`); solo se consulta Internet para saber en qué
//!   zona está el lugar.

use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use serde::Serialize;

use crate::error::{ErrorCode, JarvisError, Result};

/// Lugar resuelto.
#[derive(Debug, Clone, Serialize)]
pub struct Place {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: Option<String>,
    pub country_code: Option<String>,
    /// El usuario nombró un país y se ha usado su capital.
    pub capital_of_country: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalTime {
    pub location: String,
    pub timezone: String,
    /// Hora local legible, p. ej. «19:05».
    pub time: String,
    /// Fecha local legible, p. ej. «sábado 11 de octubre de 2026».
    pub date: String,
    pub iso: String,
    pub utc_offset: String,
    /// El país tiene varias zonas horarias: la hora es la de la capital.
    pub multiple_timezones: bool,
}

/// Capitales por código ISO 3166-1 (nombre buscable en el geocodificador).
pub fn capital(cc: &str) -> Option<&'static str> {
    Some(match cc.to_ascii_uppercase().as_str() {
        "AD" => "Andorra la Vella",
        "AE" => "Abu Dhabi",
        "AF" => "Kabul",
        "AG" => "Saint John's",
        "AL" => "Tirana",
        "AM" => "Yerevan",
        "AO" => "Luanda",
        "AR" => "Buenos Aires",
        "AT" => "Vienna",
        "AU" => "Canberra",
        "AZ" => "Baku",
        "BA" => "Sarajevo",
        "BB" => "Bridgetown",
        "BD" => "Dhaka",
        "BE" => "Brussels",
        "BF" => "Ouagadougou",
        "BG" => "Sofia",
        "BH" => "Manama",
        "BI" => "Gitega",
        "BJ" => "Porto-Novo",
        "BN" => "Bandar Seri Begawan",
        "BO" => "La Paz",
        "BR" => "Brasília",
        "BS" => "Nassau",
        "BT" => "Thimphu",
        "BW" => "Gaborone",
        "BY" => "Minsk",
        "BZ" => "Belmopan",
        "CA" => "Ottawa",
        "CD" => "Kinshasa",
        "CF" => "Bangui",
        "CG" => "Brazzaville",
        "CH" => "Bern",
        "CI" => "Yamoussoukro",
        "CL" => "Santiago",
        "CM" => "Yaoundé",
        "CN" => "Beijing",
        "CO" => "Bogotá",
        "CR" => "San José",
        "CU" => "Havana",
        "CV" => "Praia",
        "CY" => "Nicosia",
        "CZ" => "Prague",
        "DE" => "Berlin",
        "DJ" => "Djibouti",
        "DK" => "Copenhagen",
        "DM" => "Roseau",
        "DO" => "Santo Domingo",
        "DZ" => "Algiers",
        "EC" => "Quito",
        "EE" => "Tallinn",
        "EG" => "Cairo",
        "ER" => "Asmara",
        "ES" => "Madrid",
        "ET" => "Addis Ababa",
        "FI" => "Helsinki",
        "FJ" => "Suva",
        "FR" => "Paris",
        "GA" => "Libreville",
        "GB" => "London",
        "GD" => "Saint George's",
        "GE" => "Tbilisi",
        "GH" => "Accra",
        "GM" => "Banjul",
        "GN" => "Conakry",
        "GQ" => "Malabo",
        "GR" => "Athens",
        "GT" => "Guatemala City",
        "GW" => "Bissau",
        "GY" => "Georgetown",
        "HN" => "Tegucigalpa",
        "HR" => "Zagreb",
        "HT" => "Port-au-Prince",
        "HU" => "Budapest",
        "ID" => "Jakarta",
        "IE" => "Dublin",
        "IL" => "Jerusalem",
        "IN" => "New Delhi",
        "IQ" => "Baghdad",
        "IR" => "Tehran",
        "IS" => "Reykjavik",
        "IT" => "Rome",
        "JM" => "Kingston",
        "JO" => "Amman",
        "JP" => "Tokyo",
        "KE" => "Nairobi",
        "KG" => "Bishkek",
        "KH" => "Phnom Penh",
        "KM" => "Moroni",
        "KN" => "Basseterre",
        "KP" => "Pyongyang",
        "KR" => "Seoul",
        "KW" => "Kuwait City",
        "KZ" => "Astana",
        "LA" => "Vientiane",
        "LB" => "Beirut",
        "LC" => "Castries",
        "LI" => "Vaduz",
        "LK" => "Colombo",
        "LR" => "Monrovia",
        "LS" => "Maseru",
        "LT" => "Vilnius",
        "LU" => "Luxembourg",
        "LV" => "Riga",
        "LY" => "Tripoli",
        "MA" => "Rabat",
        "MC" => "Monaco",
        "MD" => "Chisinau",
        "ME" => "Podgorica",
        "MG" => "Antananarivo",
        "MK" => "Skopje",
        "ML" => "Bamako",
        "MM" => "Naypyidaw",
        "MN" => "Ulaanbaatar",
        "MR" => "Nouakchott",
        "MT" => "Valletta",
        "MU" => "Port Louis",
        "MV" => "Malé",
        "MW" => "Lilongwe",
        "MX" => "Mexico City",
        "MY" => "Kuala Lumpur",
        "MZ" => "Maputo",
        "NA" => "Windhoek",
        "NE" => "Niamey",
        "NG" => "Abuja",
        "NI" => "Managua",
        "NL" => "Amsterdam",
        "NO" => "Oslo",
        "NP" => "Kathmandu",
        "NZ" => "Wellington",
        "OM" => "Muscat",
        "PA" => "Panama City",
        "PE" => "Lima",
        "PG" => "Port Moresby",
        "PH" => "Manila",
        "PK" => "Islamabad",
        "PL" => "Warsaw",
        "PR" => "San Juan",
        "PS" => "Ramallah",
        "PT" => "Lisbon",
        "PY" => "Asunción",
        "QA" => "Doha",
        "RO" => "Bucharest",
        "RS" => "Belgrade",
        "RU" => "Moscow",
        "RW" => "Kigali",
        "SA" => "Riyadh",
        "SB" => "Honiara",
        "SC" => "Victoria",
        "SD" => "Khartoum",
        "SE" => "Stockholm",
        "SG" => "Singapore",
        "SI" => "Ljubljana",
        "SK" => "Bratislava",
        "SL" => "Freetown",
        "SM" => "San Marino",
        "SN" => "Dakar",
        "SO" => "Mogadishu",
        "SR" => "Paramaribo",
        "SS" => "Juba",
        "SV" => "San Salvador",
        "SY" => "Damascus",
        "SZ" => "Mbabane",
        "TD" => "N'Djamena",
        "TG" => "Lomé",
        "TH" => "Bangkok",
        "TJ" => "Dushanbe",
        "TL" => "Dili",
        "TM" => "Ashgabat",
        "TN" => "Tunis",
        "TR" => "Ankara",
        "TT" => "Port of Spain",
        "TW" => "Taipei",
        "TZ" => "Dodoma",
        "UA" => "Kyiv",
        "UG" => "Kampala",
        "US" => "Washington",
        "UY" => "Montevideo",
        "UZ" => "Tashkent",
        "VA" => "Vatican City",
        "VC" => "Kingstown",
        "VE" => "Caracas",
        "VN" => "Hanoi",
        "WS" => "Apia",
        "YE" => "Sanaa",
        "ZA" => "Pretoria",
        "ZM" => "Lusaka",
        "ZW" => "Harare",
        _ => return None,
    })
}

/// Países con varias zonas horarias en su territorio principal o habitual.
pub fn has_multiple_timezones(cc: &str) -> bool {
    matches!(
        cc.to_ascii_uppercase().as_str(),
        "US" | "CA"
            | "MX"
            | "BR"
            | "RU"
            | "AU"
            | "ID"
            | "KZ"
            | "MN"
            | "CD"
            | "CL"
            | "ES"
            | "PT"
            | "EC"
            | "KI"
            | "FM"
            | "PG"
            | "NZ"
            | "FR"
            | "GB"
            | "DK"
            | "NL"
    )
}

const WEEKDAYS: [&str; 7] = [
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
    "domingo",
];
const MONTHS: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

/// Hora local en una zona IANA, en español.
pub fn local_time(
    location: &str,
    tz_name: &str,
    now: DateTime<Utc>,
    multiple: bool,
) -> Result<LocalTime> {
    let tz: Tz = tz_name.parse().map_err(|_| {
        JarvisError::new(
            ErrorCode::NotFound,
            format!("Zona horaria desconocida: {tz_name}"),
        )
    })?;
    Ok(describe(
        location,
        tz_name,
        now.with_timezone(&tz),
        multiple,
    ))
}

/// Hora del propio equipo.
pub fn machine_time(now: DateTime<chrono::Local>) -> LocalTime {
    describe("aquí (hora de este equipo)", "local", now, false)
}

fn describe<T: chrono::TimeZone>(
    location: &str,
    tz_name: &str,
    t: DateTime<T>,
    multiple: bool,
) -> LocalTime
where
    T::Offset: std::fmt::Display,
{
    LocalTime {
        location: location.to_string(),
        timezone: tz_name.to_string(),
        time: t.format("%H:%M").to_string(),
        date: format!(
            "{} {} de {} de {}",
            WEEKDAYS[t.weekday().num_days_from_monday() as usize],
            t.day(),
            MONTHS[t.month0() as usize],
            t.year()
        ),
        iso: t.to_rfc3339(),
        utc_offset: t.format("%:z").to_string(),
        multiple_timezones: multiple,
    }
}

/// Nombre legible a partir de un resultado del geocodificador.
pub fn display_name(r: &serde_json::Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for k in ["name", "admin1", "country"] {
        if let Some(v) = r[k].as_str() {
            if !parts.contains(&v) {
                parts.push(v);
            }
        }
    }
    parts.join(", ")
}

/// Convierte un resultado del geocodificador en [`Place`].
pub fn place_from(r: &serde_json::Value, capital_of_country: bool) -> Place {
    Place {
        name: display_name(r),
        latitude: r["latitude"].as_f64().unwrap_or_default(),
        longitude: r["longitude"].as_f64().unwrap_or_default(),
        timezone: r["timezone"].as_str().map(str::to_string),
        country_code: r["country_code"].as_str().map(str::to_string),
        capital_of_country,
    }
}

/// ¿El resultado es un país (y no una ciudad)? Códigos GeoNames `PCL*`.
pub fn is_country(r: &serde_json::Value) -> bool {
    r["feature_code"]
        .as_str()
        .is_some_and(|c| c.starts_with("PCL"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn local_time_in_spanish() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 17, 5, 0).unwrap();
        let t = local_time("Tokio, Japón", "Asia/Tokyo", now, false).unwrap();
        assert_eq!(t.time, "02:05");
        assert_eq!(t.date, "lunes 12 de octubre de 2026");
        assert_eq!(t.utc_offset, "+09:00");
        let t = local_time("Madrid", "Europe/Madrid", now, true).unwrap();
        assert_eq!(t.time, "19:05");
        assert!(local_time("x", "Marte/Olimpo", now, false).is_err());
    }

    #[test]
    fn capitals_and_multi_zone_countries() {
        assert_eq!(capital("jp"), Some("Tokyo"));
        assert_eq!(capital("CO"), Some("Bogotá"));
        assert!(has_multiple_timezones("US"));
        assert!(!has_multiple_timezones("JP"));
    }
}
