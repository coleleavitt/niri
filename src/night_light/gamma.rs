//! Color temperature to gamma ramp conversion for niri's built-in night-light.
//!
//! Uses the Tanner Helland approximation for blackbody color temperature → RGB,
//! then generates a gamma LUT ramp matching the layout expected by `GammaProps::set_gamma`
//! in `src/backend/tty.rs`.

/// Convert a color temperature (Kelvin) to RGB multipliers (0.0–1.0).
///
/// Valid range: 1000K – 25000K. 6500K ≈ neutral daylight (1.0, 1.0, 1.0).
///
/// Uses the Tanner Helland approximation algorithm.
pub fn temperature_to_rgb(temp: u32) -> (f64, f64, f64) {
    let temp_hundreds = temp as f64 / 100.0;

    if temp_hundreds <= 66.0 {
        let r = 1.0;
        let g = if temp_hundreds <= 1.0 {
            0.0
        } else {
            ((99.4708 * temp_hundreds.ln() - 161.11957) / 255.0).clamp(0.0, 1.0)
        };
        let b = if temp_hundreds <= 19.0 {
            0.0
        } else {
            let t = temp_hundreds - 10.0;
            if t <= 0.0 {
                0.0
            } else {
                ((t.ln() * 138.51773 - 305.0448) / 255.0).clamp(0.0, 1.0)
            }
        };
        (r, g, b)
    } else {
        let r = ((329.69873 * (temp_hundreds - 60.0).powf(-0.13320476)) / 255.0).clamp(0.0, 1.0);
        let g = ((288.12216 * (temp_hundreds - 60.0).powf(-0.07551485)) / 255.0).clamp(0.0, 1.0);
        let b = 1.0;
        (r, g, b)
    }
}

/// Generate a gamma LUT ramp for a given output.
///
/// The ramp format matches what `GammaProps::set_gamma` in `src/backend/tty.rs` expects:
/// a `Vec<u16>` of length `gamma_size * 3`.
///
/// # Buffer layout: `[RED | GREEN | BLUE]`
///
/// In `tty.rs:2666-2669`, the split and zip logic is:
/// ```text
/// let (red, rest) = gamma.split_at(gamma_size);     // segment 1
/// let (blue, green) = rest.split_at(gamma_size);    // segment 2, segment 3
/// zip(zip(red, blue), green)
///     .map(|((&red, &green), &blue)| drm_color_lut { red, green, blue, .. })
/// ```
///
/// Despite the misleading variable names (`blue`/`green` are swapped in the split),
/// the destructuring reveals that:
/// - Segment 1 → `drm_color_lut.red`
/// - Segment 2 → `drm_color_lut.green`
/// - Segment 3 → `drm_color_lut.blue`
///
/// So the actual buffer layout is **[RED, GREEN, BLUE]** — standard consecutive channels.
pub fn generate_gamma_ramp(gamma_size: u32, temperature: u32, brightness: f64) -> Vec<u16> {
    let size = gamma_size as usize;
    assert!(size >= 2, "gamma_size must be at least 2");

    let (r_factor, g_factor, b_factor) = temperature_to_rgb(temperature);

    let mut ramp = vec![0u16; size * 3];
    let (red, rest) = ramp.split_at_mut(size);
    let (green, blue) = rest.split_at_mut(size);

    for i in 0..size {
        let value = i as f64 / (size - 1) as f64;
        red[i] = (value * r_factor * brightness * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16;
        green[i] = (value * g_factor * brightness * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16;
        blue[i] = (value * b_factor * brightness * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16;
    }

    ramp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_6500k_is_neutral() {
        let (r, g, b) = temperature_to_rgb(6500);
        // At 6500K (daylight), all channels should be approximately 1.0.
        assert!(
            (r - 1.0).abs() < 0.02,
            "red at 6500K should be ~1.0, got {r}"
        );
        assert!(
            (g - 1.0).abs() < 0.02,
            "green at 6500K should be ~1.0, got {g}"
        );
        assert!(
            (b - 1.0).abs() < 0.02,
            "blue at 6500K should be ~1.0, got {b}"
        );
    }

    #[test]
    fn test_3500k_warm() {
        let (r, g, b) = temperature_to_rgb(3500);
        // Warm temperature: red > green > blue, blue significantly reduced.
        assert!(r > g, "at 3500K, red ({r}) should be > green ({g})");
        assert!(g > b, "at 3500K, green ({g}) should be > blue ({b})");
        assert!(
            b < 0.7,
            "at 3500K, blue ({b}) should be significantly reduced"
        );
        assert!(r > 0.95, "at 3500K, red ({r}) should be near 1.0");
    }

    #[test]
    fn test_1000k_extreme_warm() {
        let (r, g, b) = temperature_to_rgb(1000);
        // Very warm: red at max, green very low, blue zero.
        assert_eq!(r, 1.0);
        assert!(g < 0.5, "at 1000K, green ({g}) should be very low");
        assert_eq!(b, 0.0, "at 1000K, blue should be 0");
    }

    #[test]
    fn test_10000k_cool() {
        let (r, g, b) = temperature_to_rgb(10000);
        // Cool temperature: blue at max, red and green reduced.
        assert_eq!(b, 1.0);
        assert!(r < 1.0, "at 10000K, red ({r}) should be < 1.0");
        assert!(g < 1.0, "at 10000K, green ({g}) should be < 1.0");
    }

    #[test]
    fn test_generate_ramp_neutral_is_linear() {
        let size = 256u32;
        let ramp = generate_gamma_ramp(size, 6500, 1.0);
        assert_eq!(ramp.len(), size as usize * 3);

        let s = size as usize;
        let red = &ramp[..s];
        let green = &ramp[s..s * 2];
        let blue = &ramp[s * 2..];

        // At 6500K + brightness 1.0, the ramp should be approximately linear.
        // The Tanner Helland approximation isn't perfectly neutral at 6500K
        // (blue ≈ 0.98, green ≈ 0.997), so we allow ~2% tolerance.
        assert!(
            red[s - 1] >= 64000,
            "red max should be ~65535, got {}",
            red[s - 1]
        );
        assert!(
            green[s - 1] >= 64000,
            "green max should be ~65535, got {}",
            green[s - 1]
        );
        assert!(
            blue[s - 1] >= 64000,
            "blue max should be ~65535, got {}",
            blue[s - 1]
        );

        // First element should be 0.
        assert_eq!(red[0], 0);
        assert_eq!(green[0], 0);
        assert_eq!(blue[0], 0);

        // Check approximate linearity for red (which is exactly 1.0 at 6500K):
        // middle value should be ~32768.
        let mid = s / 2;
        let expected_mid = 65535.0 * (mid as f64 / (s - 1) as f64);
        assert!(
            (red[mid] as f64 - expected_mid).abs() < 500.0,
            "red midpoint should be ~{expected_mid}, got {}",
            red[mid]
        );
    }

    #[test]
    fn test_ramp_monotonically_increasing() {
        // Test with a warm temperature to exercise non-trivial scaling.
        let size = 64u32;
        let ramp = generate_gamma_ramp(size, 3500, 0.8);
        let s = size as usize;

        let red = &ramp[..s];
        let green = &ramp[s..s * 2];
        let blue = &ramp[s * 2..];

        for channel_name in ["red", "green", "blue"] {
            let channel = match channel_name {
                "red" => red,
                "green" => green,
                "blue" => blue,
                _ => unreachable!(),
            };

            for i in 1..s {
                assert!(
                    channel[i] >= channel[i - 1],
                    "{channel_name} ramp is not monotonically increasing at index {i}: {} < {}",
                    channel[i],
                    channel[i - 1]
                );
            }
        }
    }

    #[test]
    fn test_brightness_scales_output() {
        let size = 256u32;
        let ramp_full = generate_gamma_ramp(size, 4500, 1.0);
        let ramp_half = generate_gamma_ramp(size, 4500, 0.5);
        let s = size as usize;

        // The half-brightness ramp maximum should be approximately half of full.
        let full_max = ramp_full[s - 1] as f64;
        let half_max = ramp_half[s - 1] as f64;
        let ratio = half_max / full_max;
        assert!(
            (ratio - 0.5).abs() < 0.01,
            "brightness 0.5 should halve the ramp, ratio = {ratio}"
        );
    }

    #[test]
    fn test_ramp_layout_matches_tty_expectations() {
        // Verify the ramp layout is [R, G, B] by checking that at a warm temperature,
        // the first segment (red) has higher values than the third segment (blue).
        let size = 256u32;
        let ramp = generate_gamma_ramp(size, 3000, 1.0);
        let s = size as usize;

        let red_max = ramp[s - 1];
        let green_max = ramp[s * 2 - 1];
        let blue_max = ramp[s * 3 - 1];

        assert!(
            red_max > green_max,
            "at 3000K, red max ({red_max}) should be > green max ({green_max})"
        );
        assert!(
            green_max > blue_max,
            "at 3000K, green max ({green_max}) should be > blue max ({blue_max})"
        );
    }
}
