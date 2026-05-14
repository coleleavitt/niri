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
}

impl MergeWith<NightLightPart> for NightLight {
    fn merge_with(&mut self, part: &NightLightPart) {
        if part.off {
            self.off = true;
        }
        merge_clone_opt!((self, part), latitude, longitude);
        merge_clone!((self, part), temperature_day, temperature_night, transition_duration, brightness_night);
    }
}
