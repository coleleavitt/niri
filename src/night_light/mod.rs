pub mod gamma;
pub mod solar;

use std::time::{SystemTime, UNIX_EPOCH};

use niri_config::NightLight as NightLightConfig;

/// Night light state and logic.
///
/// Computes the current color temperature based on solar elevation
/// and handles smooth transitions between day/night temperatures.
pub struct NightLight {
    /// Configuration
    latitude: f64,
    longitude: f64,
    temp_day: u32,
    temp_night: u32,
    transition_duration_mins: u32,
    brightness_night: f64,

    /// Current interpolated temperature
    current_temp: u32,
    /// Current brightness
    current_brightness: f64,
    /// Whether an external gamma client has taken over
    external_gamma_active: bool,
    /// Whether the feature is enabled
    enabled: bool,
}

impl NightLight {
    /// Create from config. Returns None if night-light section is disabled or
    /// lat/lon not provided.
    pub fn new(config: &NightLightConfig) -> Option<Self> {
        if config.off {
            return None;
        }

        let latitude = config.latitude?;
        let longitude = config.longitude?;

        Some(Self {
            latitude,
            longitude,
            temp_day: config.temperature_day,
            temp_night: config.temperature_night,
            transition_duration_mins: config.transition_duration,
            brightness_night: config.brightness_night,
            current_temp: config.temperature_day,
            current_brightness: 1.0,
            external_gamma_active: false,
            enabled: true,
        })
    }

    /// Called periodically (every ~60 seconds) to update the color temperature.
    /// Returns Some((temperature, brightness)) if the gamma should be updated,
    /// None if no change needed.
    pub fn tick(&mut self) -> Option<(u32, f64)> {
        if !self.enabled || self.external_gamma_active {
            return None;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let elevation = solar::solar_elevation(now, self.latitude, self.longitude);
        let target_temp = self.elevation_to_temperature(elevation);
        let target_brightness = self.elevation_to_brightness(elevation);

        // Only return Some if values changed
        if target_temp != self.current_temp
            || (target_brightness - self.current_brightness).abs() > 0.001
        {
            self.current_temp = target_temp;
            self.current_brightness = target_brightness;
            Some((target_temp, target_brightness))
        } else {
            None
        }
    }

    /// Notify that an external wlr-gamma-control client connected for an output.
    pub fn set_external_gamma_active(&mut self, active: bool) {
        self.external_gamma_active = active;
    }

    /// Whether we should be applying gamma ourselves.
    pub fn should_apply(&self) -> bool {
        self.enabled && !self.external_gamma_active
    }

    /// Get the current temperature for generating gamma ramps.
    pub fn current_temp(&self) -> u32 {
        self.current_temp
    }

    /// Get the current brightness.
    pub fn current_brightness(&self) -> f64 {
        self.current_brightness
    }

    /// Update from config (e.g. on config reload).
    pub fn update_config(&mut self, config: &NightLightConfig) {
        self.enabled = !config.off;

        if let Some(lat) = config.latitude {
            self.latitude = lat;
        }
        if let Some(lon) = config.longitude {
            self.longitude = lon;
        }

        self.temp_day = config.temperature_day;
        self.temp_night = config.temperature_night;
        self.transition_duration_mins = config.transition_duration;
        self.brightness_night = config.brightness_night;
    }

    /// Map solar elevation to color temperature.
    ///
    /// Uses thresholds from redshift:
    /// - Elevation > 3° → full daytime temperature
    /// - Elevation between -3° and 3° → transitioning (linear interpolation)
    /// - Elevation < -3° → full nighttime temperature
    fn elevation_to_temperature(&self, elevation: f64) -> u32 {
        const HIGH_ELEV: f64 = 3.0; // degrees above horizon = full day
        const LOW_ELEV: f64 = -3.0; // degrees below horizon = full night

        if elevation >= HIGH_ELEV {
            self.temp_day
        } else if elevation <= LOW_ELEV {
            self.temp_night
        } else {
            // Linear interpolation
            let t = (elevation - LOW_ELEV) / (HIGH_ELEV - LOW_ELEV);
            let temp = self.temp_night as f64 + t * (self.temp_day as f64 - self.temp_night as f64);
            temp.round() as u32
        }
    }

    /// Map solar elevation to brightness.
    ///
    /// Same thresholds as temperature:
    /// - Elevation > 3° → brightness 1.0 (full)
    /// - Elevation between -3° and 3° → transitioning (linear interpolation)
    /// - Elevation < -3° → brightness_night
    fn elevation_to_brightness(&self, elevation: f64) -> f64 {
        const HIGH_ELEV: f64 = 3.0;
        const LOW_ELEV: f64 = -3.0;

        if elevation >= HIGH_ELEV {
            1.0
        } else if elevation <= LOW_ELEV {
            self.brightness_night
        } else {
            // Linear interpolation
            let t = (elevation - LOW_ELEV) / (HIGH_ELEV - LOW_ELEV);
            self.brightness_night + t * (1.0 - self.brightness_night)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a NightLight with typical defaults for testing.
    fn test_night_light() -> NightLight {
        NightLight {
            latitude: 45.0,
            longitude: -93.0,
            temp_day: 6500,
            temp_night: 4000,
            transition_duration_mins: 30,
            brightness_night: 0.8,
            current_temp: 6500,
            current_brightness: 1.0,
            external_gamma_active: false,
            enabled: true,
        }
    }

    #[test]
    fn elevation_full_day() {
        let nl = test_night_light();
        assert_eq!(nl.elevation_to_temperature(10.0), 6500);
        assert_eq!(nl.elevation_to_temperature(3.0), 6500);
        assert!((nl.elevation_to_brightness(10.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn elevation_full_night() {
        let nl = test_night_light();
        assert_eq!(nl.elevation_to_temperature(-10.0), 4000);
        assert_eq!(nl.elevation_to_temperature(-3.0), 4000);
        assert!((nl.elevation_to_brightness(-10.0) - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn elevation_transition_midpoint() {
        let nl = test_night_light();
        // At elevation 0° we should be exactly halfway between night and day
        let temp = nl.elevation_to_temperature(0.0);
        assert_eq!(temp, 5250); // (4000 + 6500) / 2 = 5250
        let brightness = nl.elevation_to_brightness(0.0);
        assert!((brightness - 0.9).abs() < 0.001); // (0.8 + 1.0) / 2 = 0.9
    }

    #[test]
    fn should_apply_when_enabled() {
        let nl = test_night_light();
        assert!(nl.should_apply());
    }

    #[test]
    fn should_not_apply_when_disabled() {
        let mut nl = test_night_light();
        nl.enabled = false;
        assert!(!nl.should_apply());
    }

    #[test]
    fn should_not_apply_when_external_gamma() {
        let mut nl = test_night_light();
        nl.set_external_gamma_active(true);
        assert!(!nl.should_apply());
    }

    #[test]
    fn tick_returns_none_when_disabled() {
        let mut nl = test_night_light();
        nl.enabled = false;
        assert!(nl.tick().is_none());
    }

    #[test]
    fn tick_returns_none_when_external_gamma() {
        let mut nl = test_night_light();
        nl.set_external_gamma_active(true);
        assert!(nl.tick().is_none());
    }
}
