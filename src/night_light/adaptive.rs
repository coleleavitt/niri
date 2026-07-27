use niri_config::night_light::AdaptiveNightLight;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveUpdate {
    pub backlight: Option<f64>,
    pub gamma_brightness: f64,
    /// Smoothed room colour temperature in kelvin, when a sensor supplies one.
    pub ambient_temperature: Option<f64>,
}

#[derive(Debug, Default)]
pub struct AdaptiveController {
    smoothed_lux: Option<f64>,
    smoothed_temperature: Option<f64>,
    current_backlight: Option<f64>,
}

impl AdaptiveController {
    pub fn tick(
        &mut self,
        config: &AdaptiveNightLight,
        ambient_lux: Option<f64>,
        ambient_temperature: Option<f64>,
    ) -> AdaptiveUpdate {
        if !config.on {
            return AdaptiveUpdate {
                backlight: None,
                gamma_brightness: 1.0,
                ambient_temperature: None,
            };
        }

        // Colour temperature is independent of the backlight curve: a room can
        // be dim and warm, or dim and daylit, and the screen should follow.
        let temperature = ambient_temperature
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|kelvin| self.smooth_temperature(kelvin, config.smoothing));

        let Some(lux) = ambient_lux.filter(|value| value.is_finite() && *value >= 0.0) else {
            return AdaptiveUpdate {
                backlight: None,
                gamma_brightness: 1.0,
                ambient_temperature: temperature,
            };
        };

        let lux = self.smooth_lux(lux, config.smoothing);
        let target = target_backlight(config, lux);
        let backlight = self.backlight_update(target, config.hysteresis);
        let gamma_brightness = gamma_brightness(config, target);

        AdaptiveUpdate {
            backlight,
            gamma_brightness,
            ambient_temperature: temperature,
        }
    }

    fn smooth_temperature(&mut self, kelvin: f64, smoothing: f64) -> f64 {
        let smoothing = smoothing.clamp(0.0, 1.0);
        let smoothed = match self.smoothed_temperature {
            Some(previous) => previous + smoothing * (kelvin - previous),
            None => kelvin,
        };
        self.smoothed_temperature = Some(smoothed);
        smoothed
    }

    fn smooth_lux(&mut self, lux: f64, smoothing: f64) -> f64 {
        let smoothing = smoothing.clamp(0.0, 1.0);
        let smoothed = match self.smoothed_lux {
            Some(previous) => previous + smoothing * (lux - previous),
            None => lux,
        };
        self.smoothed_lux = Some(smoothed);
        smoothed
    }

    fn backlight_update(&mut self, target: f64, hysteresis: f64) -> Option<f64> {
        let hysteresis = hysteresis.max(0.0);
        if self
            .current_backlight
            .is_some_and(|current| (target - current).abs() < hysteresis)
        {
            return None;
        }

        self.current_backlight = Some(target);
        Some(target)
    }

    /// Forgets the last applied backlight value.
    ///
    /// `backlight_update` records the target optimistically, so without this a
    /// single failed write would be suppressed by hysteresis forever and the
    /// backlight would never be driven again.
    pub fn invalidate_backlight(&mut self) {
        self.current_backlight = None;
    }
}

fn target_backlight(config: &AdaptiveNightLight, lux: f64) -> f64 {
    let low_lux = config.low_lux.max(0.0);
    let high_lux = config.high_lux.max(low_lux + 1.0);
    let min_backlight = config.min_backlight.clamp(0.0, 1.0);
    let max_backlight = config.max_backlight.clamp(min_backlight, 1.0);

    let position = lux_position(lux, low_lux, high_lux);
    min_backlight + position * (max_backlight - min_backlight)
}

fn gamma_brightness(config: &AdaptiveNightLight, backlight: f64) -> f64 {
    let gamma_min = config.gamma_min.clamp(0.0, 1.0);
    let min_backlight = config.min_backlight.clamp(0.0, 1.0);
    let max_backlight = config.max_backlight.clamp(min_backlight, 1.0);
    let threshold = config.gamma_dim_below.clamp(min_backlight, max_backlight);

    if backlight >= threshold {
        return 1.0;
    }

    if threshold <= min_backlight {
        return gamma_min;
    }

    let position = ((backlight - min_backlight) / (threshold - min_backlight)).clamp(0.0, 1.0);
    gamma_min + position * (1.0 - gamma_min)
}

fn lux_position(lux: f64, low_lux: f64, high_lux: f64) -> f64 {
    if lux <= low_lux {
        return 0.0;
    }
    if lux >= high_lux {
        return 1.0;
    }

    let low = (low_lux + 1.0).ln();
    let high = (high_lux + 1.0).ln();
    let value = (lux + 1.0).ln();

    ((value - low) / (high - low)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use niri_config::night_light::AdaptiveNightLight;

    use super::AdaptiveController;

    #[test]
    fn dim_room_maps_to_low_backlight_and_gamma_fallback() {
        let mut controller = AdaptiveController::default();
        let config = AdaptiveNightLight {
            on: true,
            low_lux: 2.0,
            high_lux: 500.0,
            min_backlight: 0.1,
            max_backlight: 1.0,
            gamma_dim_below: 0.25,
            gamma_min: 0.6,
            smoothing: 1.0,
            hysteresis: 0.01,
            ..Default::default()
        };

        let update = controller.tick(&config, Some(0.5), None);

        assert_eq!(update.backlight, Some(0.1));
        assert!((update.gamma_brightness - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn bright_room_maps_to_full_backlight_without_gamma_dimming() {
        let mut controller = AdaptiveController::default();
        let config = AdaptiveNightLight {
            on: true,
            low_lux: 2.0,
            high_lux: 500.0,
            min_backlight: 0.1,
            max_backlight: 1.0,
            gamma_dim_below: 0.25,
            gamma_min: 0.6,
            smoothing: 1.0,
            hysteresis: 0.01,
            ..Default::default()
        };

        let update = controller.tick(&config, Some(700.0), None);

        assert_eq!(update.backlight, Some(1.0));
        assert!((update.gamma_brightness - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn small_backlight_changes_are_held_by_hysteresis() {
        let mut controller = AdaptiveController::default();
        let config = AdaptiveNightLight {
            on: true,
            low_lux: 0.0,
            high_lux: 1000.0,
            min_backlight: 0.1,
            max_backlight: 1.0,
            smoothing: 1.0,
            hysteresis: 0.05,
            ..Default::default()
        };

        let first = controller.tick(&config, Some(100.0), None);
        let second = controller.tick(&config, Some(110.0), None);

        assert!(first.backlight.is_some());
        assert_eq!(second.backlight, None);
    }

    #[test]
    fn ambient_temperature_is_smoothed_and_survives_missing_lux() {
        let mut controller = AdaptiveController::default();
        let config = AdaptiveNightLight {
            on: true,
            smoothing: 0.5,
            ..Default::default()
        };

        let first = controller.tick(&config, Some(50.0), Some(6500.0));
        assert_eq!(first.ambient_temperature, Some(6500.0));

        // Half-way toward the new reading, not a jump.
        let second = controller.tick(&config, Some(50.0), Some(2500.0));
        assert_eq!(second.ambient_temperature, Some(4500.0));

        // Temperature still reports even when the lux sensor has nothing.
        let third = controller.tick(&config, None, Some(2500.0));
        assert_eq!(third.ambient_temperature, Some(3500.0));
        assert_eq!(third.backlight, None);

        // No temperature sensor means no opinion, not a default.
        assert_eq!(
            controller.tick(&config, Some(50.0), None).ambient_temperature,
            None
        );
    }

    #[test]
    fn invalidating_backlight_forces_a_retry_after_a_failed_write() {
        let mut controller = AdaptiveController::default();
        let config = AdaptiveNightLight {
            on: true,
            low_lux: 0.0,
            high_lux: 1000.0,
            min_backlight: 0.1,
            max_backlight: 1.0,
            smoothing: 1.0,
            hysteresis: 0.05,
            ..Default::default()
        };

        let first = controller.tick(&config, Some(100.0), None);
        assert!(first.backlight.is_some());

        // Pretend the write failed: without invalidation hysteresis would hide
        // the identical target forever, leaving the backlight stuck.
        assert_eq!(controller.tick(&config, Some(100.0), None).backlight, None);
        controller.invalidate_backlight();

        assert_eq!(
            controller.tick(&config, Some(100.0), None).backlight,
            first.backlight
        );
    }
}
