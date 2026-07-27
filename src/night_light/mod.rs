pub mod adaptive;
pub mod backlight;
pub mod gamma;
pub mod solar;
pub mod sysfs;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use adaptive::AdaptiveController;
use backlight::Backlight;
use niri_config::night_light::AdaptiveNightLight;
use niri_config::NightLight as NightLightConfig;

/// Night light state and logic.
///
/// Computes the current color temperature based on solar elevation
/// and handles smooth transitions between day/night temperatures.
pub struct NightLight {
    /// Configuration
    latitude: Option<f64>,
    longitude: Option<f64>,
    temp_day: u32,
    temp_night: u32,
    transition_duration_mins: u32,
    brightness_night: f64,
    adaptive_config: AdaptiveNightLight,
    adaptive: AdaptiveController,
    /// Resolved backlight output, created on first use.
    backlight: Option<Backlight>,

    /// Current interpolated temperature
    current_temp: u32,
    /// Current brightness
    current_brightness: f64,
    /// Whether an external gamma client has taken over
    external_gamma_active: bool,
    /// Whether the feature is enabled
    enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightLightUpdate {
    pub temperature: u32,
    pub brightness: f64,
    pub gamma_changed: bool,
    pub backlight: Option<f64>,
}

impl NightLight {
    pub fn new(config: &NightLightConfig) -> Option<Self> {
        if !config_enabled(config) {
            return None;
        }

        Some(Self {
            latitude: config.latitude,
            longitude: config.longitude,
            temp_day: config.temperature_day,
            temp_night: config.temperature_night,
            transition_duration_mins: config.transition_duration,
            brightness_night: config.brightness_night,
            adaptive_config: config.adaptive.clone(),
            adaptive: AdaptiveController::default(),
            backlight: None,
            current_temp: config.temperature_day,
            current_brightness: 1.0,
            external_gamma_active: false,
            enabled: true,
        })
    }

    pub fn tick(
        &mut self,
        ambient_lux: Option<f64>,
        ambient_temperature: Option<f64>,
    ) -> Option<NightLightUpdate> {
        if !self.enabled {
            return None;
        }

        let adaptive = self
            .adaptive
            .tick(&self.adaptive_config, ambient_lux, ambient_temperature);
        let (solar_temp, solar_brightness) =
            if let (Some(latitude), Some(longitude)) = (self.latitude, self.longitude) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();

                let elevation = solar::solar_elevation(now, latitude, longitude);
                (
                    self.elevation_to_temperature(elevation),
                    self.elevation_to_brightness(elevation),
                )
            } else {
                (self.temp_day, 1.0)
            };

        // A measured room temperature wins over the sun: the point is to match
        // the light you are actually sitting in. temperature-day/night become
        // the bounds of what the screen will do rather than the endpoints of a
        // solar curve.
        let target_temp = match adaptive.ambient_temperature {
            Some(kelvin) => self.clamp_temperature(kelvin),
            None => solar_temp,
        };
        let target_brightness = (solar_brightness * adaptive.gamma_brightness).clamp(0.0, 1.0);

        let gamma_changed = !self.external_gamma_active
            && (target_temp != self.current_temp
                || (target_brightness - self.current_brightness).abs() > 0.001);

        if gamma_changed {
            self.current_temp = target_temp;
            self.current_brightness = target_brightness;
        }

        (gamma_changed || adaptive.backlight.is_some()).then_some(NightLightUpdate {
            temperature: target_temp,
            brightness: target_brightness,
            gamma_changed,
            backlight: adaptive.backlight,
        })
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
        self.enabled = config_enabled(config);
        self.latitude = config.latitude;
        self.longitude = config.longitude;

        self.temp_day = config.temperature_day;
        self.temp_night = config.temperature_night;
        self.transition_duration_mins = config.transition_duration;
        self.brightness_night = config.brightness_night;
        if self.adaptive_config != config.adaptive {
            // Device selection may have changed; re-resolve on next use.
            self.backlight = None;
        }
        self.adaptive_config = config.adaptive.clone();
    }

    pub fn read_ambient_lux(&self) -> Option<f64> {
        if !self.enabled || !self.adaptive_config.on {
            return None;
        }

        sysfs::read_ambient_lux(&self.adaptive_config)
    }

    /// Reads the room's measured colour temperature, if a sensor supplies one.
    pub fn read_ambient_temperature(&self) -> Option<f64> {
        if !self.enabled || !self.adaptive_config.on {
            return None;
        }

        sysfs::read_ambient_temperature(&self.adaptive_config)
    }

    /// Restricts a measured room temperature to the configured screen range.
    fn clamp_temperature(&self, kelvin: f64) -> u32 {
        let low = self.temp_night.min(self.temp_day);
        let high = self.temp_night.max(self.temp_day);
        (kelvin.round() as i64).clamp(low as i64, high as i64) as u32
    }

    /// Applies a backlight ratio, resolving the device on first use.
    ///
    /// The device handle is cached so the sysfs-denied -> logind fallback is
    /// decided once rather than re-probed on every tick.
    pub fn set_backlight_ratio(&mut self, ratio: f64) -> io::Result<()> {
        if self.backlight.is_none() {
            match Backlight::new(&self.adaptive_config) {
                Ok(backlight) => self.backlight = Some(backlight),
                Err(err) => {
                    self.adaptive.invalidate_backlight();
                    return Err(err);
                }
            }
        }

        let result = self.backlight.as_mut().unwrap().set_ratio(ratio);
        if result.is_err() {
            // The controller recorded this ratio as applied before we tried it,
            // so clear it or hysteresis suppresses every future attempt and the
            // backlight stays wherever it was after one transient failure.
            self.adaptive.invalidate_backlight();
            self.backlight = None;
        }

        result
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

fn config_enabled(config: &NightLightConfig) -> bool {
    !config.off && (config.adaptive.on || (config.latitude.is_some() && config.longitude.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a NightLight with typical defaults for testing.
    fn test_night_light() -> NightLight {
        NightLight {
            latitude: Some(45.0),
            longitude: Some(-93.0),
            temp_day: 6500,
            temp_night: 4000,
            transition_duration_mins: 30,
            brightness_night: 0.8,
            adaptive_config: AdaptiveNightLight::default(),
            adaptive: AdaptiveController::default(),
            backlight: None,
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
        assert!(nl.tick(None, None).is_none());
    }

    #[test]
    fn tick_returns_none_when_external_gamma() {
        let mut nl = test_night_light();
        nl.set_external_gamma_active(true);
        assert!(nl.tick(None, None).is_none());
    }

    #[test]
    fn adaptive_night_light_can_start_without_coordinates() {
        let config = NightLightConfig {
            adaptive: AdaptiveNightLight {
                on: true,
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(NightLight::new(&config).is_some());
    }

    #[test]
    fn measured_room_temperature_drives_the_screen_within_configured_bounds() {
        let config = NightLightConfig {
            temperature_day: 6500,
            temperature_night: 2700,
            adaptive: AdaptiveNightLight {
                on: true,
                smoothing: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut nl = NightLight::new(&config).unwrap();

        // A daylit room pulls the screen neutral...
        let update = nl.tick(Some(200.0), Some(6400.0)).unwrap();
        assert_eq!(update.temperature, 6400);

        // ...a warm bulb pulls it warm.
        let update = nl.tick(Some(200.0), Some(2900.0)).unwrap();
        assert_eq!(update.temperature, 2900);

        // Readings outside the configured range are clamped, not obeyed:
        // a camera pointed at a blue screen should not blow past the bounds.
        let update = nl.tick(Some(200.0), Some(9000.0)).unwrap();
        assert_eq!(update.temperature, 6500);
        let update = nl.tick(Some(200.0), Some(1200.0)).unwrap();
        assert_eq!(update.temperature, 2700);
    }

    #[test]
    fn without_a_temperature_sensor_the_sun_still_decides() {
        let config = NightLightConfig {
            temperature_day: 6500,
            temperature_night: 2700,
            adaptive: AdaptiveNightLight {
                on: true,
                smoothing: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut nl = NightLight::new(&config).unwrap();

        // No coordinates and no sensor: temperature-day is the constant.
        let update = nl.tick(Some(200.0), None).unwrap();
        assert_eq!(update.temperature, 6500);
    }

    #[test]
    fn adaptive_tick_updates_backlight_while_external_gamma_is_active() {
        let mut nl = NightLight::new(&NightLightConfig {
            adaptive: AdaptiveNightLight {
                on: true,
                smoothing: 1.0,
                low_lux: 2.0,
                high_lux: 500.0,
                min_backlight: 0.1,
                max_backlight: 1.0,
                gamma_min: 0.6,
                gamma_dim_below: 0.25,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        nl.set_external_gamma_active(true);

        let update = nl.tick(Some(0.5), None).unwrap();

        assert_eq!(update.backlight, Some(0.1));
        assert!(!update.gamma_changed);
    }
}
