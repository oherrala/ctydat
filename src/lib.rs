//! # A parser for CTY.DAT file format
//!
//! <https://www.country-files.com/cty-dat-format/>

use std::char;
use std::io;
use std::path::Path;

use chumsky::prelude::*;
use chumsky::text::newline;
use tracing::instrument;

#[derive(Debug)]
pub struct Ctydat {
    pub values: Vec<Country>,
}

impl Ctydat {
    #[instrument(skip(s))]
    pub fn from_str(s: &str) -> io::Result<Ctydat> {
        let ts = std::time::Instant::now();
        let values = parser()
            .parse(s)
            .map_err(|err| {
                tracing::error!("Parse errors found: {:?}", err);
                io::Error::new(io::ErrorKind::InvalidInput, "parse error")
            })?;
        tracing::debug!("Parsed {} records in {} ms.", values.len(), ts.elapsed().as_millis());
        Ok(Ctydat { values })
    }

    #[instrument(fields(path = path.as_ref().display().to_string()))]
    pub fn from_path<P: AsRef<Path>>(path: P) -> io::Result<Ctydat> {
        let s = std::fs::read(path)?;
        let s = std::str::from_utf8(&s).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Self::from_str(s)
    }
}

/// Single country from CTY.DAT file
///
/// <https://www.country-files.com/cty-dat-format/>
#[derive(Debug)]
pub struct Country {
    /// Country Name
    pub country_name: String,
    /// CQ Zone
    pub cq_zone: u8,
    /// ITU Zone
    pub itu_zone: u8,
    /// 2-letter continent abbreviation
    pub continent: String,
    /// Latitude in degrees, + for North
    pub latitude: f32,
    /// Longitude in degrees, + for West
    pub longitude: f32,
    /// Local time offset from GMT
    pub time_offset: f32,
    /// Primary DXCC Prefix
    pub primary_prefix: String,
    /// Alias DXCC prefixes including the primary one
    ///
    /// If an alias prefix is a [Prefix::Callsign], this indicates that the
    /// prefix is to be treated as a full callsign, i.e. must be an exact match.
    pub prefix_list: Vec<Prefix>,
}

/// A single prefix or exact callsign
#[derive(Debug, Clone)]
pub enum Prefix {
    /// A single prefix
    Callsign(String, Option<Vec<Override>>),
    /// A full callsign
    Prefix(String, Option<Vec<Override>>),
}

#[derive(Debug, Clone)]
pub enum Override {
    CqZone(u8),
    ItuZone(u8),
    Coordinates((f32, f32)),
    Continent(String),
    TimeOffset(f32),
}

fn parser() -> impl Parser<char, Vec<Country>, Error = Simple<char>> {
    let ascii_not_comma = |c: &char| c.is_ascii() && !c.is_control() && *c != ':';
    let ascii_float = |c: &char| c.is_ascii_digit() || *c == '-' || *c == '.';

    let country_name = filter(ascii_not_comma)
        .repeated()
        .at_least(4 /* Peru, Fiji */)
        .labelled("Country name")
        .collect();

    let cq_zone = text::digits(10)
        .labelled("CQ zone")
        .try_map(|s: String, span| s.parse().map_err(|e| Simple::custom(span, format!("{e}"))));

    let itu_zone = text::digits(10)
        .labelled("ITU zone")
        .try_map(|s: String, span| s.parse().map_err(|e| Simple::custom(span, format!("{e}"))));

    let continent = filter(ascii_not_comma)
        .repeated()
        .exactly(2)
        .labelled("Continent")
        .collect();

    let latitude = filter(ascii_float)
        .repeated()
        .labelled("Latitude")
        .collect()
        .try_map(|s: String, span| s.parse().map_err(|e| Simple::custom(span, format!("{e}"))));

    let longitude = filter(ascii_float)
        .repeated()
        .labelled("Longitude")
        .collect()
        .try_map(|s: String, span| s.parse().map_err(|e| Simple::custom(span, format!("{e}"))));

    let time_offset = filter(ascii_float)
        .repeated()
        .labelled("Time offset")
        .collect()
        .try_map(|s: String, span| s.parse().map_err(|e| Simple::custom(span, format!("{e}"))));

    let primary_prefix = filter(ascii_not_comma)
        .repeated()
        .at_least(1 /* K, F */)
        .labelled("Primary prefix")
        .collect();

    let prefix_list = {
        let prefix = filter(|c: &char| c.is_ascii_alphanumeric() || *c == '/')
            .repeated()
            .labelled("DXCC Prefix")
            .collect();
        let callsign = just("=").ignore_then(prefix).labelled("Exact callsign");

        // The following special characters can be applied after an alias prefix:
        // (#)      Override CQ Zone
        // [#]      Override ITU Zone
        // <#/#>    Override latitude/longitude
        // {aa}     Override Continent
        // ~#~      Override local time offset from GMT
        let over_ride = cq_zone
            .delimited_by(just('['), just(']'))
            .map(Override::CqZone)
            .or(itu_zone
                .delimited_by(just('('), just(')'))
                .map(Override::ItuZone))
            .or(continent
                .delimited_by(just('{'), just('}'))
                .map(Override::Continent))
            .or(time_offset
                .delimited_by(just('~'), just('~'))
                .map(Override::TimeOffset))
            .or(latitude.then_ignore(just('/')).then(longitude).delimited_by(just('<'), just('>')).map(Override::Coordinates));

        let one_dxcc = callsign
            .then(over_ride.repeated())
            .map(|(c, o)| Prefix::Callsign(c, empty_is_none(o)))
            .or(prefix
                .then(over_ride.repeated())
                .map(|(p, o)| Prefix::Prefix(p, empty_is_none(o))));

        one_dxcc
            .padded()
            .separated_by(just(','))
            .then_ignore(just(';'))
    };

    let one_country = country_name
        .then_ignore(just(':').padded())
        .then(cq_zone)
        .then_ignore(just(':').padded())
        .then(itu_zone)
        .then_ignore(just(':').padded())
        .then(continent)
        .then_ignore(just(':').padded())
        .then(latitude)
        .then_ignore(just(':').padded())
        .then(longitude)
        .then_ignore(just(':').padded())
        .then(time_offset)
        .then_ignore(just(':').padded())
        .then(primary_prefix)
        .then_ignore(just(':').padded())
        .then(prefix_list)
        .then_ignore(newline())
        .map(|value| {
            let (value, prefix_list) = value;
            let (value, primary_prefix) = value;
            let (value, time_offset) = value;
            let (value, longitude) = value;
            let (value, latitude) = value;
            let (value, continent) = value;
            let (value, itu_zone) = value;
            let (value, cq_zone) = value;
            let country_name = value;
            Country {
                country_name,
                cq_zone,
                itu_zone,
                continent,
                latitude,
                longitude,
                time_offset,
                primary_prefix,
                prefix_list,
            }
        });

    one_country.repeated()
}

fn empty_is_none<V>(i: Vec<V>) -> Option<Vec<V>> {
    if i.is_empty() {
        None
    } else {
        Some(i)
    }
}

#[cfg(test)]
mod tests {
    use crate::Ctydat;

    const FINLAND: &str = r##"Finland:                  15:  18:  EU:   61.38:   -24.82:    -2.0:  OH:
        OF,OG,OH,OI,OJ,=OH/RX3AMI/LH,
        =OF100FI/1/LH,=OF1AD/S,=OF1LD/S,=OF1TX/S,=OH0HG/1,=OH0J/1,=OH0JJS/1,=OH0MDR/1,=OH0MRR/1,=OH1AD/S,
        =OH1AF/LH,=OH1AH/LH,=OH1AH/LT,=OH1AM/LH,=OH1BGG/S,=OH1BGG/SA,=OH1BS/SA,=OH1CM/S,=OH1F/LGT,
        =OH1F/LH,=OH1FJ/S,=OH1FJ/SA,=OH1KW/S,=OH1KW/SA,=OH1LD/S,=OH1LEO/S,=OH1MLZ/SA,=OH1NR/S,=OH1OD/S,
        =OH1OD/SA,=OH1PP/S,=OH1PV/S,=OH1S/S,=OH1SJ/S,=OH1SJ/SA,=OH1SM/S,=OH1TX/S,=OH1TX/SA,=OH1UH/S,
        =OH1XW/S,=OI1AXA/S,=OI1AY/S,=OI1SWM/S,
        =OF2BNX/SA,=OG2O/YL,=OH0AM/2,=OH0BT/2,=OH0HG/2,=OH0SCA/2,=OH2AAF/S,=OH2AAF/SA,=OH2AAV/S,
        =OH2AN/SUB,=OH2AUE/S,=OH2AUE/SA,=OH2AY/S,=OH2BAX/S,=OH2BMB/S,=OH2BMB/SA,=OH2BNB/SA,=OH2BNX/S,
        =OH2BNX/SA,=OH2BQP/S,=OH2BXT/S,=OH2C/S,=OH2EO/S,=OH2ET/LH,=OH2ET/LS,=OH2ET/S,=OH2FBX/S,=OH2FBX/SA,
        =OH2HK/S,=OH2HZ/S,=OH2MEE/S,=OH2MEE/SA,=OH2MH/S,=OH2MO/S,=OH2MO/SA,=OH2NAS/S,=OH2NAS/SA,=OH2NM/LH,
        =OH2PO/S,=OH2PO/SA,=OH2S/S,=OH2S/SA,=OH2XL/S,=OH2XMP/S,=OH2ZL/SA,=OH2ZY/S,=OI2ABG/S,
        =OF3HHO/S,=OF3KRB/S,=OG3X/LH,=OH0MZA/3,=OH3A/LH,=OH3ABN/S,=OH3ACA/S,=OH3AG/LH,=OH3CT/S,=OH3CT/SA,
        =OH3FJQ/S,=OH3FJQ/SA,=OH3GDO/LH,=OH3GQM/S,=OH3HB/S,=OH3HB/SA,=OH3HHO/S,=OH3HHO/SA,=OH3IH/S,
        =OH3IH/SA,=OH3IS/S,=OH3KRB/S,=OH3KRB/SA,=OH3LB/S,=OH3LB/SA,=OH3LS/S,=OH3MY/S,=OH3MY/SA,=OH3N/S,
        =OH3NOB/S,=OH3NVK/S,=OH3R/SA,=OH3SUF/JOTA,=OH3TAM/LH,=OH3VV/S,=OH3W/S,=OH3WR/SA,=OI3SVM/S,
        =OI3SVM/SA,=OI3V/LH,=OI3V/S,=OI3V/SA,=OI3W/LGT,=OI3W/LH,
        =OG0V/4,=OH0I/4,=OH0V/4,=OH4FSL/SA,=OH4N/S,=OH4SG/S,=OI4JM/S,=OI4JM/SA,=OI4PM/S,
        =OF200AD/LS,=OF200AD/S,=OF5AD/S,=OG5A/LS,=OG5A/S,=OH0AW/5,=OH5A/S,=OH5AA/LS,=OH5AD/LS,=OH5AD/S,
        =OH5B/LH,=OH5EAB/S,=OH5EAB/SA,=OH5GOE/S,=OH5J/S,=OH5J/SA,=OH5JJL/S,=OH5K/S,=OH5LP/S,=OH5LP/SA,
        =OH5R/S,=OH5ZB/S,=OI5AY/LH,=OI5AY/SA,=OI5PRM/SA,
        =OF6FSQ/S,=OF6NL/SA,=OF6QR/S,=OG6M/S,=OH0Y/6,=OH2Y/6/LH,=OH6AC/LH,=OH6ADHD/LH,=OH6AG/S,=OH6AR/LH,
        =OH6CT/S,=OH6CT/SA,=OH6EFH/SA,=OH6EOG/SA,=OH6FA/S,=OH6FA/SA,=OH6FMG/LH,=OH6FSQ/S,=OH6G/S,
        =OH6GSR/S,=OH6HGW/S,=OH6K/S,=OH6MH/S,=OH6NL/S,=OH6NL/SA,=OH6NR/LGT,=OH6NR/LH,=OH6NZ/SA,=OH6OG/SA,
        =OH6OS/S,=OH6OT/S,=OH6P/SA,=OH6PA/S,=OH6QR/S,=OH6QR/SA,=OH6RJ/S,=OH6UW/S,=OH6VM/S,=OI6AY/LH,
        =OI6MPK/SA,=OI6SP/S,=OI6SP/SA,
        =OH7AB/S,=OH7AX/S,=OH7BD/S,=OH7ND/S,=OH7NE/S,=OH7QA/S,=OH7QA/SA,=OH7SV/SA,=OH7UE/S,=OH7VL/S,
        =OH7XI/S,=OI7AX/S,
        =OH0SCA/8,=OH8AAU/LH,=OH8FCK/S,=OH8FCK/SA,=OH8KN/S,=OH8KN/SA,=OH8UV/SA,=OI8VK/S,
        =OH0KAG/9,=OH9AR/S,=OH9TM/S,=OH9TO/S;
    "##;

    #[test]
    fn test_parser() {
        let ctydat = Ctydat::from_str(FINLAND).unwrap();
        let cty = ctydat.values.first().unwrap();
        dbg!(&cty);
        assert_eq!(cty.country_name, "Finland");
        assert_eq!(cty.cq_zone, 15);
        assert_eq!(cty.itu_zone, 18);
        assert_eq!(cty.latitude, 61.38);
        assert_eq!(cty.longitude, -24.82);
        assert_eq!(cty.time_offset, -2.0);
    }
}
