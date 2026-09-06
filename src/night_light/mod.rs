pub mod adaptive;
pub mod backlight;
pub mod gamma;
pub mod solar;
pub mod sysfs;

use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    elevation_day: f64,
    elevation_night: f64,
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
    /// Set when the ramps on the outputs no longer match `current_*` and must be re-applied
    /// even if the target did not change: a new output, an external gamma client letting go,
    /// or a config reload.
    needs_reapply: bool,
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
            elevation_day: config.elevation_day,
            elevation_night: config.elevation_night,
            brightness_night: config.brightness_night,
            adaptive_config: config.adaptive.clone(),
            adaptive: AdaptiveController::default(),
            backlight: None,
            current_temp: config.temperature_day,
            current_brightness: 1.0,
            external_gamma_active: false,
            enabled: true,
            needs_reapply: true,
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

        // The sun sets the ceiling, the room can only pull it warmer. Without coordinates the
        // ceiling is temperature-day, so the screen simply follows the room between the two
        // bounds. With coordinates, night still means warm even in a daylight-coloured room
        // (a webcam under a cool LED bulb would otherwise keep the screen at 6500K at 2am), and
        // a warm lamp during the day still warms the screen beyond the solar curve.
        let mut target_temp = match adaptive.ambient_temperature {
            Some(kelvin) => self.clamp_temperature(kelvin).min(solar_temp),
            None => solar_temp,
        };

        // A dim room warms the screen too. Blue light is far more glaring against dark
        // surroundings, and a webcam's colour reading says nothing about that: a dim room
        // under daylight from a window still reads 6300K. Map low-lux..high-lux onto
        // temperature-night..temperature-day and let it cap the target like the sun does.
        if self.adaptive_config.temperature_from_lux {
            if let Some(position) = adaptive.lux_position {
                let night = self.temp_night.min(self.temp_day) as f64;
                let day = self.temp_night.max(self.temp_day) as f64;
                let from_lux = (night + position * (day - night)).round() as u32;
                target_temp = target_temp.min(from_lux);
            }
        }
        let target_brightness = (solar_brightness * adaptive.gamma_brightness).clamp(0.0, 1.0);

        let gamma_changed = !self.external_gamma_active
            && (self.needs_reapply
                || target_temp != self.current_temp
                || (target_brightness - self.current_brightness).abs() > 0.001);

        if gamma_changed {
            self.current_temp = target_temp;
            self.current_brightness = target_brightness;
            self.needs_reapply = false;
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
        if self.external_gamma_active && !active {
            // The client reset the ramps to identity on its way out; ours must go back.
            self.needs_reapply = true;
        }
        self.external_gamma_active = active;
    }

    /// Whether the output ramps are known to be out of date.
    pub fn needs_reapply(&self) -> bool {
        self.enabled && !self.external_gamma_active && self.needs_reapply
    }

    /// Forces the next tick to re-apply the current ramps.
    ///
    /// Call this whenever something reset the hardware gamma behind our back, e.g. a newly
    /// connected output (`connector_connected` resets GAMMA_LUT) or a session resume.
    pub fn request_reapply(&mut self) {
        self.needs_reapply = true;
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
    ///
    /// Returns `true` when the feature was just switched off, so the caller can reset the
    /// output ramps; without that the last tint stays on screen until the compositor exits.
    pub fn update_config(&mut self, config: &NightLightConfig) -> bool {
        let was_enabled = self.enabled;
        self.enabled = config_enabled(config);
        self.needs_reapply = true;
        self.latitude = config.latitude;
        self.longitude = config.longitude;

        self.temp_day = config.temperature_day;
        self.temp_night = config.temperature_night;
        self.elevation_day = config.elevation_day;
        self.elevation_night = config.elevation_night;
        self.brightness_night = config.brightness_night;
        if self.adaptive_config != config.adaptive {
            // Device selection may have changed; re-resolve on next use.
            self.backlight = None;
        }
        self.adaptive_config = config.adaptive.clone();

        was_enabled && !self.enabled
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

        let backlight = self.backlight.as_mut().unwrap();
        let hold = Duration::from_secs(self.adaptive_config.manual_hold_secs);
        if backlight.manual_override_active(hold) {
            // Forget the target so the controller offers it again once the hold expires,
            // instead of hysteresis treating the never-applied value as current.
            self.adaptive.invalidate_backlight();
            return Ok(());
        }

        let result = backlight.set_ratio(ratio);
        if result.is_err() {
            // The controller recorded this ratio as applied before we tried it,
            // so clear it or hysteresis suppresses every future attempt and the
            // backlight stays wherever it was after one transient failure.
            self.adaptive.invalidate_backlight();
            self.backlight = None;
        }

        result
    }

    /// Where the sun sits between full night (0.0) and full day (1.0).
    ///
    /// Full day at and above `elevation-day`, full night at and below `elevation-night`,
    /// linear in between. The defaults (3° / -6°) are redshift's: the ramp starts a little
    /// before sunset and finishes at the end of civil twilight.
    fn day_fraction(&self, elevation: f64) -> f64 {
        let low = self.elevation_night.min(self.elevation_day);
        let high = self.elevation_day.max(self.elevation_night);
        if (high - low).abs() < f64::EPSILON {
            return if elevation >= high { 1.0 } else { 0.0 };
        }
        ((elevation - low) / (high - low)).clamp(0.0, 1.0)
    }

    /// Map solar elevation to color temperature.
    fn elevation_to_temperature(&self, elevation: f64) -> u32 {
        let t = self.day_fraction(elevation);
        let temp = self.temp_night as f64 + t * (self.temp_day as f64 - self.temp_night as f64);
        temp.round() as u32
    }

    /// Map solar elevation to brightness, from `brightness_night` up to 1.0.
    fn elevation_to_brightness(&self, elevation: f64) -> f64 {
        let t = self.day_fraction(elevation);
        self.brightness_night + t * (1.0 - self.brightness_night)
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
            elevation_day: 3.0,
            elevation_night: -3.0,
            brightness_night: 0.8,
            adaptive_config: AdaptiveNightLight::default(),
            adaptive: AdaptiveController::default(),
            backlight: None,
            current_temp: 6500,
            current_brightness: 1.0,
            external_gamma_active: false,
            enabled: true,
            needs_reapply: false,
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
    fn elevation_thresholds_come_from_config_and_survive_being_swapped() {
        let mut nl = test_night_light();
        nl.elevation_day = 10.0;
        nl.elevation_night = -6.0;
        assert_eq!(nl.elevation_to_temperature(10.0), 6500);
        assert_eq!(nl.elevation_to_temperature(-6.0), 4000);
        assert_eq!(nl.elevation_to_temperature(2.0), 5250);

        // Reversed by mistake: still a sensible ramp instead of a divide-by-negative.
        nl.elevation_day = -6.0;
        nl.elevation_night = 10.0;
        assert_eq!(nl.elevation_to_temperature(2.0), 5250);
        nl.elevation_day = 0.0;
        nl.elevation_night = 0.0;
        assert_eq!(nl.elevation_to_temperature(0.1), 6500);
        assert_eq!(nl.elevation_to_temperature(-0.1), 4000);
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
    fn the_sun_caps_the_measured_room_temperature() {
        let mut nl = test_night_light();
        nl.latitude = None;
        nl.longitude = None;
        nl.adaptive_config = AdaptiveNightLight {
            on: true,
            smoothing: 1.0,
            ..Default::default()
        };

        // Pretend the solar curve says full night.
        nl.temp_day = 4000;
        nl.temp_night = 4000;
        // A 6300K room must not push the screen past the schedule...
        assert_eq!(nl.tick(Some(50.0), Some(6300.0)).unwrap().temperature, 4000);

        // ...but a warm bulb still pulls it warmer than the schedule allows for.
        nl.temp_night = 2700;
        assert_eq!(nl.tick(Some(50.0), Some(2900.0)).unwrap().temperature, 2900);
    }

    #[test]
    fn a_dim_room_warms_the_screen_when_temperature_from_lux_is_on() {
        let config = NightLightConfig {
            temperature_day: 6500,
            temperature_night: 2700,
            adaptive: AdaptiveNightLight {
                on: true,
                smoothing: 1.0,
                low_lux: 2.0,
                high_lux: 300.0,
                temperature_from_lux: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut nl = NightLight::new(&config).unwrap();

        // Pitch dark: full night temperature even though the room reads daylight.
        assert_eq!(nl.tick(Some(0.0), Some(6300.0)).unwrap().temperature, 2700);
        // Bright: the room's own reading is the only cap left.
        assert_eq!(
            nl.tick(Some(1000.0), Some(6300.0)).unwrap().temperature,
            6300
        );
        // Dim daylit room, like a laptop by a window with the blinds down: in between.
        let dim = nl.tick(Some(5.0), Some(6300.0)).unwrap().temperature;
        assert!((3000..=3600).contains(&dim), "{dim}");

        // Off by default: the same dim room stays at the room's colour.
        let off = NightLightConfig {
            adaptive: AdaptiveNightLight {
                temperature_from_lux: false,
                ..config.adaptive.clone()
            },
            ..config
        };
        let mut nl = NightLight::new(&off).unwrap();
        assert_eq!(nl.tick(Some(5.0), Some(6300.0)).unwrap().temperature, 6300);
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
    fn first_tick_applies_gamma_even_when_nothing_changed() {
        // A fresh instance starts at temperature-day / brightness 1.0, which is exactly the
        // no-op target without coordinates. The ramps must still be sent once, because the
        // hardware starts at identity and 6500K through our curve is not quite identity.
        let config = NightLightConfig {
            adaptive: AdaptiveNightLight {
                on: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut nl = NightLight::new(&config).unwrap();

        let update = nl.tick(None, None).unwrap();
        assert!(update.gamma_changed);
        assert!(nl.tick(None, None).is_none());
    }

    #[test]
    fn reapply_is_requested_after_external_gamma_releases_and_new_outputs() {
        let mut nl = test_night_light();
        nl.latitude = None;
        nl.longitude = None;
        assert!(nl.tick(None, None).is_none());

        // The client reset the ramps on its way out: ours go back without waiting for drift.
        nl.set_external_gamma_active(true);
        assert!(!nl.needs_reapply());
        nl.set_external_gamma_active(false);
        assert!(nl.needs_reapply());
        assert!(nl.tick(None, None).unwrap().gamma_changed);
        assert!(!nl.needs_reapply());

        // Same for a connector that came up with identity gamma.
        nl.request_reapply();
        assert!(nl.tick(None, None).unwrap().gamma_changed);
        assert!(nl.tick(None, None).is_none());
    }

    #[test]
    fn switching_off_reports_that_gamma_must_be_reset() {
        let on = NightLightConfig {
            adaptive: AdaptiveNightLight {
                on: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut nl = NightLight::new(&on).unwrap();
        assert!(!nl.update_config(&on));

        let off = NightLightConfig {
            off: true,
            ..on.clone()
        };
        assert!(nl.update_config(&off));
        assert!(!nl.should_apply());
        // Only the transition reports it, not every reload while off.
        assert!(!nl.update_config(&off));
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
