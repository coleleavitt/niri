//! Solar position calculations for the night-light feature.
//!
//! Implements the solar elevation algorithm needed to determine whether it's
//! day, night, or twilight at a given geographic location.

use std::f64::consts::PI;

const DEG_TO_RAD: f64 = PI / 180.0;
const RAD_TO_DEG: f64 = 180.0 / PI;

/// Calculate the solar elevation angle in degrees for a given Unix timestamp
/// and geographic coordinates.
///
/// Returns the angle in degrees above (positive) or below (negative) the
/// horizon.
///
/// Algorithm based on NOAA Solar Calculations (simplified Jean Meeus).
pub fn solar_elevation(unix_time: f64, latitude: f64, longitude: f64) -> f64 {
    // Julian date from Unix time
    let jd = unix_time / 86400.0 + 2440587.5;

    // Julian century from J2000.0
    let jc = (jd - 2451545.0) / 36525.0;

    // Geometric mean longitude of the Sun (degrees)
    let geom_mean_long = (280.46646 + jc * (36000.76983 + 0.0003032 * jc)) % 360.0;

    // Geometric mean anomaly of the Sun (degrees)
    let geom_mean_anom = 357.52911 + jc * (35999.05029 - 0.0001537 * jc);
    let geom_mean_anom_rad = geom_mean_anom * DEG_TO_RAD;

    // Eccentricity of Earth's orbit
    let ecc_earth = 0.016708634 - jc * (0.000042037 + 0.0000001267 * jc);

    // Sun equation of center
    let sun_eq_center = (1.914602 - jc * (0.004817 + 0.000014 * jc)) * geom_mean_anom_rad.sin()
        + (0.019993 - 0.000101 * jc) * (2.0 * geom_mean_anom_rad).sin()
        + 0.000289 * (3.0 * geom_mean_anom_rad).sin();

    // Sun true longitude (degrees)
    let sun_true_long = geom_mean_long + sun_eq_center;

    // Sun apparent longitude (degrees)
    let omega = 125.04 - 1934.136 * jc;
    let sun_apparent_long = sun_true_long - 0.00569 - 0.00478 * (omega * DEG_TO_RAD).sin();
    let sun_apparent_long_rad = sun_apparent_long * DEG_TO_RAD;

    // Mean obliquity of the ecliptic (degrees)
    let mean_obliq_ecliptic =
        23.0 + (26.0 + (21.448 - jc * (46.815 + jc * (0.00059 - jc * 0.001813))) / 60.0) / 60.0;

    // Corrected obliquity (degrees)
    let obliq_corr = mean_obliq_ecliptic + 0.00256 * (omega * DEG_TO_RAD).cos();
    let obliq_corr_rad = obliq_corr * DEG_TO_RAD;

    // Sun declination (radians)
    let sun_declin = (obliq_corr_rad.sin() * sun_apparent_long_rad.sin()).asin();

    // Equation of time (minutes)
    let var_y = (obliq_corr_rad / 2.0).tan().powi(2);
    let geom_mean_long_rad = geom_mean_long * DEG_TO_RAD;
    let eq_of_time = 4.0
        * RAD_TO_DEG
        * (var_y * (2.0 * geom_mean_long_rad).sin() - 2.0 * ecc_earth * geom_mean_anom_rad.sin()
            + 4.0
                * ecc_earth
                * var_y
                * geom_mean_anom_rad.sin()
                * (2.0 * geom_mean_long_rad).cos()
            - 0.5 * var_y * var_y * (4.0 * geom_mean_long_rad).sin()
            - 1.25 * ecc_earth * ecc_earth * (2.0 * geom_mean_anom_rad).sin());

    // True solar time (minutes from midnight UTC at the given longitude)
    let time_of_day_mins = ((jd - 0.5).fract()) * 1440.0;
    let true_solar_time = (time_of_day_mins + eq_of_time + 4.0 * longitude) % 1440.0;

    // Hour angle (degrees)
    let hour_angle = if true_solar_time / 4.0 < 0.0 {
        true_solar_time / 4.0 + 180.0
    } else {
        true_solar_time / 4.0 - 180.0
    };
    let hour_angle_rad = hour_angle * DEG_TO_RAD;

    // Solar elevation angle
    let lat_rad = latitude * DEG_TO_RAD;
    let sin_elevation =
        lat_rad.sin() * sun_declin.sin() + lat_rad.cos() * sun_declin.cos() * hour_angle_rad.cos();

    sin_elevation.asin() * RAD_TO_DEG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solar_elevation_known_noon() {
        // 2024-06-21 12:00 UTC at the equator, 0° longitude
        // Summer solstice noon — sun should be high
        let unix_time = 1718971200.0; // 2024-06-21 12:00:00 UTC
        let elev = solar_elevation(unix_time, 0.0, 0.0);
        // At equator on summer solstice at noon UTC/0° longitude, sun should be
        // high (around 66-67° given ~23.4° declination from equator at solstice)
        assert!(elev > 60.0, "elevation was {elev}");
        assert!(elev < 75.0, "elevation was {elev}");
    }

    #[test]
    fn test_solar_elevation_midnight() {
        // 2024-06-21 00:00 UTC at the equator, 0° longitude
        // Should be well below horizon
        let unix_time = 1718928000.0; // 2024-06-21 00:00:00 UTC
        let elev = solar_elevation(unix_time, 0.0, 0.0);
        assert!(elev < -10.0, "elevation was {elev}");
    }

    #[test]
    fn test_solar_elevation_high_latitude_summer() {
        // Near the arctic circle in summer — sun should not go far below horizon
        // 2024-06-21 00:00 UTC, 66°N, 0°E
        let unix_time = 1718928000.0;
        let elev = solar_elevation(unix_time, 66.0, 0.0);
        // Midnight sun region — elevation should be near 0 or slightly positive
        assert!(elev > -5.0, "elevation was {elev}");
    }
}
