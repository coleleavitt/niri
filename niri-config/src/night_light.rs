use std::path::PathBuf;

use crate::utils::MergeWith;

#[derive(Debug, Clone, PartialEq)]
pub struct NightLight {
    /// Whether night light is disabled
    pub off: bool,

    /// Latitude for solar calculations (-90 to 90)
    pub latitude: Option<f64>,

    /// Longitude for solar calculations (-180 to 180)
    pub longitude: Option<f64>,

    /// Daytime color temperature in Kelvin (default: 6500)
    pub temperature_day: u32,

    /// Nighttime color temperature in Kelvin (default: 3500)
    pub temperature_night: u32,

    /// Transition duration in minutes (default: 30)
    pub transition_duration: u32,

    /// Brightness at night (0.0-1.0, default: 1.0)
    pub brightness_night: f64,

    pub adaptive: AdaptiveNightLight,
}

impl Default for NightLight {
    fn default() -> Self {
        Self {
            off: false,
            latitude: None,
            longitude: None,
            temperature_day: 6500,
            temperature_night: 3500,
            transition_duration: 30,
            brightness_night: 1.0,
            adaptive: AdaptiveNightLight::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveNightLight {
    pub on: bool,
    pub sensor_path: Option<PathBuf>,
    /// File holding the room's measured colour temperature in kelvin.
    ///
    /// When set and fresh, the screen tracks the room's light instead of the
    /// sun, clamped between `temperature_night` and `temperature_day`.
    pub temperature_path: Option<PathBuf>,
    /// How old a sensor file may be before it is ignored, in seconds.
    ///
    /// A sampler that dies leaves its last value on disk forever, which is
    /// indistinguishable from a live reading without this.
    pub sensor_max_age_secs: u64,
    pub backlight_name: Option<String>,
    pub backlight_path: Option<PathBuf>,
    pub low_lux: f64,
    pub high_lux: f64,
    pub min_backlight: f64,
    pub max_backlight: f64,
    pub gamma_dim_below: f64,
    pub gamma_min: f64,
    pub smoothing: f64,
    pub hysteresis: f64,
}

impl Default for AdaptiveNightLight {
    fn default() -> Self {
        Self {
            on: false,
            sensor_path: None,
            temperature_path: None,
            sensor_max_age_secs: 300,
            backlight_name: None,
            backlight_path: None,
            low_lux: 2.0,
            high_lux: 500.0,
            min_backlight: 0.08,
            max_backlight: 1.0,
            gamma_dim_below: 0.2,
            gamma_min: 0.7,
            smoothing: 0.25,
            hysteresis: 0.02,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct NightLightPart {
    #[knuffel(child)]
    pub off: bool,

    #[knuffel(child, unwrap(argument))]
    pub latitude: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub longitude: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub temperature_day: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub temperature_night: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub transition_duration: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub brightness_night: Option<f64>,

    #[knuffel(child)]
    pub adaptive: Option<AdaptiveNightLightPart>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct AdaptiveNightLightPart {
    #[knuffel(child)]
    pub on: bool,

    #[knuffel(child)]
    pub off: bool,

    #[knuffel(child, unwrap(argument))]
    pub sensor_path: Option<PathBuf>,

    #[knuffel(child, unwrap(argument))]
    pub temperature_path: Option<PathBuf>,

    #[knuffel(child, unwrap(argument))]
    pub sensor_max_age_secs: Option<u64>,

    #[knuffel(child, unwrap(argument))]
    pub backlight_name: Option<String>,

    #[knuffel(child, unwrap(argument))]
    pub backlight_path: Option<PathBuf>,

    #[knuffel(child, unwrap(argument))]
    pub low_lux: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub high_lux: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub min_backlight: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub max_backlight: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub gamma_dim_below: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub gamma_min: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub smoothing: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub hysteresis: Option<f64>,
}

impl MergeWith<AdaptiveNightLightPart> for AdaptiveNightLight {
    fn merge_with(&mut self, part: &AdaptiveNightLightPart) {
        if part.on {
            self.on = true;
        }
        if part.off {
            self.on = false;
        }

        merge_clone_opt!(
            (self, part),
            sensor_path,
            temperature_path,
            backlight_name,
            backlight_path
        );
        merge_clone!(
            (self, part),
            sensor_max_age_secs,
            low_lux,
            high_lux,
            min_backlight,
            max_backlight,
            gamma_dim_below,
            gamma_min,
            smoothing,
            hysteresis
        );
    }
}

impl MergeWith<NightLightPart> for NightLight {
    fn merge_with(&mut self, part: &NightLightPart) {
        if part.off {
            self.off = true;
        }
        merge_clone_opt!((self, part), latitude, longitude);
        merge_clone!(
            (self, part),
            temperature_day,
            temperature_night,
            transition_duration,
            brightness_night
        );
        if let Some(adaptive) = &part.adaptive {
            self.adaptive.merge_with(adaptive);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn parse_night_light(text: &str) -> NightLight {
        let part: NightLightPart = knuffel::parse("test.kdl", text).unwrap();
        let mut night_light = NightLight::default();
        night_light.merge_with(&part);
        night_light
    }

    #[test]
    fn adaptive_eye_comfort_config_merges_from_kdl() {
        let night_light = parse_night_light(
            r#"
            adaptive {
                on
                sensor-path "/tmp/ambient-lux"
                backlight-name "intel_backlight"
                low-lux 2.0
                high-lux 500.0
                min-backlight 0.08
                max-backlight 0.9
                gamma-dim-below 0.2
                gamma-min 0.65
                smoothing 0.3
                hysteresis 0.04
            }
            "#,
        );

        assert!(night_light.adaptive.on);
        assert_eq!(
            night_light.adaptive.sensor_path.unwrap(),
            PathBuf::from("/tmp/ambient-lux")
        );
        assert_eq!(
            night_light.adaptive.backlight_name.as_deref(),
            Some("intel_backlight")
        );
        assert_eq!(night_light.adaptive.low_lux, 2.0);
        assert_eq!(night_light.adaptive.high_lux, 500.0);
        assert_eq!(night_light.adaptive.min_backlight, 0.08);
        assert_eq!(night_light.adaptive.max_backlight, 0.9);
        assert_eq!(night_light.adaptive.gamma_dim_below, 0.2);
        assert_eq!(night_light.adaptive.gamma_min, 0.65);
        assert_eq!(night_light.adaptive.smoothing, 0.3);
        assert_eq!(night_light.adaptive.hysteresis, 0.04);
    }

    #[test]
    fn ambient_temperature_sensor_merges_from_kdl() {
        let night_light = parse_night_light(
            r#"
            adaptive {
                on
                sensor-path "/tmp/ambient-lux"
                temperature-path "/tmp/ambient-temp"
                sensor-max-age-secs 90
            }
            "#,
        );

        assert_eq!(
            night_light.adaptive.temperature_path.unwrap(),
            PathBuf::from("/tmp/ambient-temp")
        );
        assert_eq!(night_light.adaptive.sensor_max_age_secs, 90);
    }

    #[test]
    fn temperature_sensor_is_opt_in() {
        let night_light = parse_night_light("adaptive { on }");
        assert!(night_light.adaptive.temperature_path.is_none());
        // A sampler that dies must not pin the screen forever.
        assert_eq!(night_light.adaptive.sensor_max_age_secs, 300);
    }
}
