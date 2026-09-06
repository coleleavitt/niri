//! Backlight output for adaptive night light.
//!
//! Writing `/sys/class/backlight/*/brightness` requires root or a udev rule
//! granting the seat's user write access. A compositor started as a normal user
//! usually has neither, so the write fails with `EACCES` on every tick and the
//! backlight silently never moves.
//!
//! logind exposes `org.freedesktop.login1.Session.SetBrightness` for exactly
//! this case and performs it unprivileged for the active session. So: write
//! sysfs directly when we may (no IPC on the hot path), and fall back to logind
//! the first time the kernel says no.

use std::io;
use std::time::{Duration, Instant};

use niri_config::night_light::AdaptiveNightLight;

use super::sysfs::{self, BacklightDevice};

/// The logind session object of the calling process.
#[cfg(feature = "dbus")]
const SESSION_PATH: &str = "/org/freedesktop/login1/session/auto";

pub struct Backlight {
    device: BacklightDevice,
    /// Set once sysfs has refused a write; we do not probe it again.
    use_logind: bool,
    /// The raw value of our last successful write, to detect someone else moving the backlight.
    last_written: Option<u64>,
    /// While set, a manual change is being honoured and we do not touch the backlight.
    manual_hold_until: Option<Instant>,
    #[cfg(feature = "dbus")]
    conn: Option<zbus::blocking::Connection>,
}

impl Backlight {
    pub fn new(config: &AdaptiveNightLight) -> io::Result<Self> {
        Ok(Self {
            device: sysfs::backlight_device(config)?,
            use_logind: false,
            last_written: None,
            manual_hold_until: None,
            #[cfg(feature = "dbus")]
            conn: None,
        })
    }

    /// Whether a manual brightness change is currently being honoured.
    ///
    /// Compares the kernel's current value with what we last wrote. A mismatch means the user
    /// (brightness keys, `brightnessctl`, ...) set it on purpose; the adaptive controller then
    /// backs off for `hold` so it does not undo them a minute later. `hold == 0` disables this.
    pub fn manual_override_active(&mut self, hold: Duration) -> bool {
        if hold.is_zero() {
            return false;
        }

        let now = Instant::now();
        if let Some(until) = self.manual_hold_until {
            if now < until {
                return true;
            }
            self.manual_hold_until = None;
        }

        let Some(last) = self.last_written else {
            return false;
        };
        match self.device.read_current() {
            // The panel may round our value; only a real difference counts.
            Ok(current) if current.abs_diff(last) > 1 => {
                debug!(
                    "night-light: backlight moved externally ({last} -> {current}), \
                     pausing adaptive backlight for {}s",
                    hold.as_secs()
                );
                self.manual_hold_until = Some(now + hold);
                // Whatever the user picked is the new baseline: only a second manual
                // change should extend the hold, not our own resumption.
                self.last_written = Some(current);
                true
            }
            _ => false,
        }
    }

    /// Applies a 0.0..=1.0 ratio of the device's maximum brightness.
    pub fn set_ratio(&mut self, ratio: f64) -> io::Result<()> {
        let target = self.device.target_for(ratio);
        let result = self.write(target);
        if result.is_ok() {
            self.last_written = Some(target);
        }
        result
    }

    fn write(&mut self, target: u64) -> io::Result<()> {
        if !self.use_logind {
            match self.device.write_sysfs(target) {
                Ok(()) => return Ok(()),
                Err(err) if is_permission_denied(&err) => {
                    debug!(
                        "night-light: backlight sysfs write denied ({err}), \
                         switching to logind"
                    );
                    self.use_logind = true;
                }
                Err(err) => return Err(err),
            }
        }

        self.set_via_logind(target)
    }

    #[cfg(feature = "dbus")]
    fn set_via_logind(&mut self, target: u64) -> io::Result<()> {
        if self.conn.is_none() {
            self.conn = Some(
                zbus::blocking::Connection::system()
                    .map_err(|err| io::Error::other(format!("no system bus: {err}")))?,
            );
        }
        let conn = self.conn.as_ref().unwrap();

        let target = u32::try_from(target).unwrap_or(u32::MAX);
        conn.call_method(
            Some("org.freedesktop.login1"),
            SESSION_PATH,
            Some("org.freedesktop.login1.Session"),
            "SetBrightness",
            &("backlight", self.device.name.as_str(), target),
        )
        .map_err(|err| io::Error::other(format!("logind SetBrightness failed: {err}")))?;

        Ok(())
    }

    #[cfg(not(feature = "dbus"))]
    fn set_via_logind(&mut self, _target: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "backlight is not writable and niri was built without D-Bus support",
        ))
    }
}

fn is_permission_denied(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn fake_device(name: &str) -> (PathBuf, Backlight) {
        let root =
            std::env::temp_dir().join(format!("niri-backlight-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("max_brightness"), "800\n").unwrap();
        fs::write(root.join("brightness"), "400\n").unwrap();
        let config = AdaptiveNightLight {
            backlight_path: Some(root.clone()),
            ..Default::default()
        };
        let backlight = Backlight::new(&config).unwrap();
        (root, backlight)
    }

    #[test]
    fn manual_change_pauses_adaptive_backlight() {
        let (root, mut backlight) = fake_device("manual");
        let hold = Duration::from_secs(60);

        // Nothing written yet: whatever the panel says is not an override.
        assert!(!backlight.manual_override_active(hold));

        backlight.set_ratio(0.5).unwrap();
        assert_eq!(fs::read_to_string(root.join("brightness")).unwrap(), "400");
        assert!(!backlight.manual_override_active(hold));

        // The user hit the brightness keys.
        fs::write(root.join("brightness"), "700\n").unwrap();
        assert!(backlight.manual_override_active(hold));
        // Still held on the next tick, and the user's value became the baseline.
        assert!(backlight.manual_override_active(hold));
        assert_eq!(backlight.last_written, Some(700));

        // Holds are opt-out.
        let mut fresh = Backlight::new(&AdaptiveNightLight {
            backlight_path: Some(root.clone()),
            ..Default::default()
        })
        .unwrap();
        fresh.set_ratio(0.25).unwrap();
        fs::write(root.join("brightness"), "700\n").unwrap();
        assert!(!fresh.manual_override_active(Duration::ZERO));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn panel_rounding_is_not_a_manual_change() {
        let (root, mut backlight) = fake_device("rounding");
        backlight.set_ratio(0.5).unwrap();
        fs::write(root.join("brightness"), "401\n").unwrap();
        assert!(!backlight.manual_override_active(Duration::from_secs(60)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hold_expires_and_adaptive_resumes() {
        let (root, mut backlight) = fake_device("expiry");
        backlight.set_ratio(0.5).unwrap();
        fs::write(root.join("brightness"), "700\n").unwrap();
        assert!(backlight.manual_override_active(Duration::from_millis(1)));
        std::thread::sleep(Duration::from_millis(5));
        assert!(!backlight.manual_override_active(Duration::from_millis(1)));
        fs::remove_dir_all(root).unwrap();
    }

    /// Drives the real panel. Ignored by default because it needs an active
    /// logind session and visibly changes the screen.
    ///
    /// Run with: cargo test --release backlight -- --ignored --nocapture
    #[test]
    #[ignore = "requires a real backlight and an active logind session"]
    fn sets_real_backlight_when_sysfs_is_not_writable() {
        let config = AdaptiveNightLight::default();
        let mut backlight = Backlight::new(&config).expect("no backlight device");

        let path = backlight.device.directory.join("brightness");
        let read_panel = || -> u64 {
            std::fs::read_to_string(&path)
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };

        let original = read_panel();
        let max = backlight.device.max_brightness;
        println!("backlight {} at {original}/{max}", backlight.device.name);

        for ratio in [0.15, 0.60] {
            let expected = backlight.device.target_for(ratio);
            backlight.set_ratio(ratio).expect("failed to set backlight");

            let actual = read_panel();
            println!(
                "ratio {ratio} -> wanted {expected}, panel reports {actual} (via {})",
                if backlight.use_logind {
                    "logind"
                } else {
                    "sysfs"
                }
            );
            assert_eq!(actual, expected);
        }

        backlight
            .set_ratio(original as f64 / max as f64)
            .expect("failed to restore backlight");
        assert_eq!(read_panel(), original);
    }
}
