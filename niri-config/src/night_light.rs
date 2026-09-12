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

    /// Solar elevation, in degrees, at and above which it is fully day (default: 3.0).
    pub elevation_day: f64,

    /// Solar elevation, in degrees, at and below which it is fully night (default: -6.0,
    /// the end of civil twilight). Between the two the temperature ramps linearly.
    pub elevation_night: f64,

    /// Brightness at night (0.0-1.0, default: 1.0)
    pub brightness_night: f64,

    /// Clock-anchored bedtime stage, warmer than the solar night.
    ///
    /// The evidence for warm screens is about the hours before sleep, not about the sun: in
    /// December the sun sets six hours before most people go to bed. From `bedtime` minus
    /// `bedtime_lead_mins` the temperature ramps over `bedtime_ramp_mins` down to
    /// `temperature_bedtime` and stays there until `wake`.
    pub bedtime: Option<ClockTime>,

    /// When the bedtime stage ends (default: 06:00).
    pub wake: ClockTime,

    /// Colour temperature during the bedtime stage in Kelvin (default: 2700).
    pub temperature_bedtime: u32,

    /// How long before `bedtime` the ramp starts, in minutes (default: 180).
    pub bedtime_lead_mins: u32,

    /// How long the ramp to `temperature_bedtime` takes, in minutes (default: 60).
    pub bedtime_ramp_mins: u32,

    pub adaptive: AdaptiveNightLight,
}

/// A wall-clock time of day, `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTime {
    pub minutes: u32,
}

impl ClockTime {
    pub const fn new(hour: u32, minute: u32) -> Self {
        Self {
            minutes: hour * 60 + minute,
        }
    }
}

impl std::str::FromStr for ClockTime {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (hour, minute) = s
            .split_once(':')
            .ok_or_else(|| format!("expected HH:MM, got {s:?}"))?;
        let hour: u32 = hour
            .trim()
            .parse()
            .map_err(|_| format!("bad hour in {s:?}"))?;
        let minute: u32 = minute
            .trim()
            .parse()
            .map_err(|_| format!("bad minute in {s:?}"))?;
        if hour > 23 || minute > 59 {
            return Err(format!("{s:?} is not a time of day"));
        }
        Ok(Self::new(hour, minute))
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::DecodeScalar<S> for ClockTime {
    fn type_check(
        type_name: &Option<knuffel::span::Spanned<knuffel::ast::TypeName, S>>,
        ctx: &mut knuffel::decode::Context<S>,
    ) {
        if let Some(type_name) = &type_name {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
    }

    fn raw_decode(
        val: &knuffel::span::Spanned<knuffel::ast::Literal, S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, knuffel::errors::DecodeError<S>> {
        match &**val {
            knuffel::ast::Literal::String(s) => match s.parse::<Self>() {
                Ok(time) => Ok(time),
                Err(err) => {
                    ctx.emit_error(knuffel::errors::DecodeError::conversion(val, err));
                    Ok(Self::new(0, 0))
                }
            },
            _ => {
                ctx.emit_error(knuffel::errors::DecodeError::unsupported(
                    val,
                    "expected a \"HH:MM\" string",
                ));
                Ok(Self::new(0, 0))
            }
        }
    }
}

impl Default for NightLight {
    fn default() -> Self {
        Self {
            off: false,
            latitude: None,
            longitude: None,
            temperature_day: 6500,
            temperature_night: 3500,
            elevation_day: 3.0,
            elevation_night: -6.0,
            brightness_night: 1.0,
            bedtime: None,
            wake: ClockTime::new(6, 0),
            temperature_bedtime: 2700,
            bedtime_lead_mins: 180,
            bedtime_ramp_mins: 60,
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
    /// How long a manual brightness change (keys, brightnessctl, ...) pauses the adaptive
    /// backlight, in seconds. `0` means the sensor always wins.
    ///
    /// The backlight is read back before every write; if it is not where we left it, someone
    /// else set it on purpose and fighting them minutes later is the worst possible outcome.
    pub manual_hold_secs: u64,
    /// Also warm the screen as the room gets darker: `low-lux` maps to `temperature-dim`,
    /// `high-lux` to `temperature-day`, and the result caps the target like the sun does.
    pub temperature_from_lux: bool,
    /// The warm end of the lux mapping in Kelvin. `None` means `temperature-night`.
    ///
    /// A dark room in the afternoon should get a *dim* screen first and only a mildly warm
    /// one; the clock and the sun own the bedtime-warm end.
    pub temperature_dim: Option<u32>,
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
            manual_hold_secs: 600,
            temperature_from_lux: false,
            temperature_dim: None,
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
    pub elevation_day: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub elevation_night: Option<f64>,

    /// Accepted for compatibility; the transition is defined by the elevation angles now.
    #[knuffel(child, unwrap(argument))]
    pub transition_duration: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub brightness_night: Option<f64>,

    #[knuffel(child, unwrap(argument))]
    pub bedtime: Option<ClockTime>,

    #[knuffel(child, unwrap(argument))]
    pub wake: Option<ClockTime>,

    #[knuffel(child, unwrap(argument))]
    pub temperature_bedtime: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub bedtime_lead_mins: Option<u32>,

    #[knuffel(child, unwrap(argument))]
    pub bedtime_ramp_mins: Option<u32>,

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

    #[knuffel(child, unwrap(argument))]
    pub manual_hold_secs: Option<u64>,

    #[knuffel(child)]
    pub temperature_from_lux: bool,

    #[knuffel(child, unwrap(argument))]
    pub temperature_dim: Option<u32>,
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
            hysteresis,
            manual_hold_secs
        );
        if part.temperature_from_lux {
            self.temperature_from_lux = true;
        }
        merge_clone_opt!((self, part), temperature_dim);
    }
}

impl MergeWith<NightLightPart> for NightLight {
    fn merge_with(&mut self, part: &NightLightPart) {
        if part.off {
            self.off = true;
        }
        merge_clone_opt!((self, part), latitude, longitude, bedtime);
        merge_clone!(
            (self, part),
            temperature_day,
            temperature_night,
            elevation_day,
            elevation_night,
            brightness_night,
            wake,
            temperature_bedtime,
            bedtime_lead_mins,
            bedtime_ramp_mins
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
    fn bedtime_stage_parses_clock_times() {
        let night_light = parse_night_light(
            r#"
            bedtime "23:30"
            wake "06:45"
            temperature-bedtime 2000
            bedtime-lead-mins 120
            bedtime-ramp-mins 45
            adaptive { temperature-from-lux; temperature-dim 3400; }
            "#,
        );
        assert_eq!(night_light.bedtime, Some(ClockTime::new(23, 30)));
        assert_eq!(night_light.wake, ClockTime::new(6, 45));
        assert_eq!(night_light.temperature_bedtime, 2000);
        assert_eq!(night_light.bedtime_lead_mins, 120);
        assert_eq!(night_light.bedtime_ramp_mins, 45);
        assert_eq!(night_light.adaptive.temperature_dim, Some(3400));

        let defaults = parse_night_light("");
        assert_eq!(defaults.bedtime, None);
        assert_eq!(defaults.wake, ClockTime::new(6, 0));
        assert_eq!(defaults.adaptive.temperature_dim, None);

        let bad: Result<NightLightPart, _> = knuffel::parse("test.kdl", r#"bedtime "25:00""#);
        assert!(bad.is_err());
        let bad: Result<NightLightPart, _> = knuffel::parse("test.kdl", "bedtime 2330");
        assert!(bad.is_err());
    }

    #[test]
    fn temperature_sensor_is_opt_in() {
        let night_light = parse_night_light("adaptive { on; }");
        assert!(night_light.adaptive.temperature_path.is_none());
        // A sampler that dies must not pin the screen forever.
        assert_eq!(night_light.adaptive.sensor_max_age_secs, 300);
    }
}
