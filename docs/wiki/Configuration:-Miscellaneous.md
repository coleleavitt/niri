This page documents all top-level options that don't otherwise have dedicated pages.

Here are all of these options at a glance:

```kdl
spawn-at-startup "waybar"
spawn-at-startup "alacritty"
spawn-sh-at-startup "qs -c ~/source/qs/MyAwesomeShell"

prefer-no-csd

screenshot-path "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png"

environment {
    QT_QPA_PLATFORM "wayland"
    DISPLAY null
}

cursor {
    xcursor-theme "breeze_cursors"
    xcursor-size 48

    hide-when-typing
    hide-after-inactive-ms 1000
}

night-light {
    latitude 42.3314
    longitude -83.0458
    temperature-day 6500
    temperature-night 3500
    brightness-night 0.9

    adaptive {
        on
        sensor-path "$XDG_RUNTIME_DIR/niri-ambient-lux"
        backlight-name "intel_backlight"
        low-lux 2.0
        high-lux 500.0
        min-backlight 0.08
        max-backlight 1.0
        gamma-dim-below 0.2
        gamma-min 0.7
        smoothing 0.25
        hysteresis 0.02
    }
}

overview {
    zoom 0.5
    backdrop-color "#262626"

    workspace-shadow {
        // off
        softness 40
        spread 10
        offset x=0 y=10
        color "#00000050"
    }
}

xwayland-satellite {
    // off
    path "xwayland-satellite"
}

clipboard {
    disable-primary
}

hotkey-overlay {
    skip-at-startup
    hide-not-bound
}

config-notification {
    disable-failed
}

blur {
    // off
    passes 3
    offset 3.0
    noise 0.02
    saturation 1.5
}
```

### `spawn-at-startup`

Add lines like this to spawn processes at niri startup.

`spawn-at-startup` accepts a path to the program binary as the first argument, followed by arguments to the program.

This option works the same way as the [`spawn` key binding action](./Configuration:-Key-Bindings.md#spawn), so please read about all its subtleties there.

```kdl
spawn-at-startup "waybar"
spawn-at-startup "alacritty"
```

Note that running niri as a systemd session supports xdg-desktop-autostart out of the box, which may be more convenient to use.
Thanks to this, apps that you configured to autostart in GNOME will also "just work" in niri, without any manual `spawn-at-startup` configuration.

### `spawn-sh-at-startup`

<sup>Since: 25.08</sup>

Add lines like this to run shell commands at niri startup.

The argument is a single string that is passed verbatim to `sh`.
You can use shell variables, pipelines, `~` expansion and everything else as expected.

See detailed description in the docs for the [`spawn-sh` key binding action](./Configuration:-Key-Bindings.md#spawn-sh).

```kdl
// Pass all arguments in the same string.
spawn-sh-at-startup "qs -c ~/source/qs/MyAwesomeShell"
```

### `prefer-no-csd`

This flag will make niri ask the applications to omit their client-side decorations.

If an application will specifically ask for CSD, the request will be honored.
Additionally, clients will be informed that they are tiled, removing some rounded corners.

With `prefer-no-csd` set, applications that negotiate server-side decorations through the xdg-decoration protocol will have focus ring and border drawn around them *without* a solid colored background.

> [!NOTE]
> Unlike most other options, changing `prefer-no-csd` will not entirely affect already running applications.
> It will make some windows rectangular, but won't remove the title bars.
> This mainly has to do with niri working around a [bug in SDL2](https://github.com/libsdl-org/SDL/issues/8173) that prevents SDL2 applications from starting.
>
> Restart applications after changing `prefer-no-csd` in the config to fully apply it.

```kdl
prefer-no-csd
```

### `screenshot-path`

Set the path where screenshots are saved.
A `~` at the front will be expanded to the home directory.

The path is formatted with `strftime(3)` to give you the screenshot date and time.

Niri will create the last folder of the path if it doesn't exist.

```kdl
screenshot-path "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png"
```

You can also set this option to `null` to disable saving screenshots to disk.

```kdl
screenshot-path null
```

### `environment`

Override environment variables for processes spawned by niri.

```kdl
environment {
    // Set a variable like this:
    // QT_QPA_PLATFORM "wayland"

    // Remove a variable by using null as the value:
    // DISPLAY null
}
```

Note that these variables do not propagate to the systemd global environment, so tools and applications started by systemd do not see them.
In particular, if you start a desktop shell like DankMaterialShell through systemd, then use its built-in application launcher, the apps won't see these environment variables.

If you want all processes to see the environment variables, you can set them in your login shell config instead (i.e. `~/.bash_profile`).
The `niri-session` shell script runs through the login shell and imports all environment variables to systemd before starting niri.
Keep in mind that all compositors will see variables set in the login shell, not just niri.

### `cursor`

Change the theme and size of the cursor as well as set the `XCURSOR_THEME` and `XCURSOR_SIZE` environment variables.

```kdl
cursor {
    xcursor-theme "breeze_cursors"
    xcursor-size 48
}
```

#### `hide-when-typing`

<sup>Since: 0.1.10</sup>

If set, hides the cursor when pressing a key on the keyboard.

> [!NOTE]
> This setting might interfere with games running in Wine in native Wayland mode that use mouselook, such as first-person games.
> If your character's point of view jumps down when you press a key and move the mouse simultaneously, try disabling this setting.

```kdl
cursor {
    hide-when-typing
}
```

#### `hide-after-inactive-ms`

<sup>Since: 0.1.10</sup>

If set, the cursor will automatically hide once this number of milliseconds passes since the last cursor movement.

```kdl
cursor {
    // Hide the cursor after one second of inactivity.
    hide-after-inactive-ms 1000
}
```

### `night-light`

Built-in night light can adjust output gamma from the sun position, using `latitude` and `longitude` to choose between the daytime and nighttime color temperatures.

The optional `adaptive` block reads ambient light from a Linux IIO illuminance sensor, or from an explicit `sensor-path` file containing a lux value. It adjusts the laptop backlight first; when the target backlight is below `gamma-dim-below`, it also applies a small gamma brightness reduction down to `gamma-min`.

Niri does not read camera frames inside the compositor. On systems without an IIO illuminance sensor, run `niri-camera-lux` as a separate helper and point `sensor-path` at its output file.

```kdl
night-light {
    latitude 42.3314
    longitude -83.0458
    temperature-day 6500
    temperature-night 3500
    brightness-night 0.9

    adaptive {
        on
        // Omit sensor-path to use the first detected IIO illuminance sensor.
        sensor-path "/sys/bus/iio/devices/iio:device0/in_illuminance_input"

        // Omit backlight-name to use the first detected backlight device.
        backlight-name "intel_backlight"

        low-lux 2.0
        high-lux 500.0
        min-backlight 0.08
        max-backlight 1.0
        gamma-dim-below 0.2
        gamma-min 0.7
        smoothing 0.25
        hysteresis 0.02
    }
}
```

Backlight writes go to `/sys/class/backlight` when that file is writable. It usually isn't for a normal user session, so niri falls back to logind's `org.freedesktop.login1.Session.SetBrightness`, which is performed unprivileged for the active session. No udev rule is required.

#### Matching the room's colour temperature

By default the screen temperature follows the sun between `temperature-day` and `temperature-night`. If a sensor can report the room's actual colour temperature, point `temperature-path` at a file containing that value in kelvin and the screen will track the light you are sitting in instead: a warm bulb warms the screen, daylight leaves it neutral.

With `temperature-path` set, `temperature-day` and `temperature-night` stop being the endpoints of a solar curve and become the bounds the measured value is clamped into. Set them to the warmest and coolest screen you are willing to accept.

```kdl
night-light {
    // The screen will stay between these two, following the room.
    temperature-night 2700
    temperature-day 6500

    adaptive {
        on
        sensor-path "$XDG_RUNTIME_DIR/niri-ambient-lux"
        temperature-path "$XDG_RUNTIME_DIR/niri-ambient-temp"

        // Ignore either sensor file once it stops being updated. 0 disables
        // the check, for sensors that legitimately never change.
        sensor-max-age-secs 300
    }
}
```

Readings outside 1000K-25000K are rejected as sampler noise. The value is smoothed by `smoothing`, so pointing a camera at a passing headlight will not flip the screen.

`sensor-max-age-secs` matters more than it looks: an external sampler that dies leaves its last value on disk forever, which is otherwise indistinguishable from a live reading, and the screen would stay pinned to whatever the room looked like when the sampler stopped.

#### Camera Lux Helper

`niri-camera-lux` samples one GREY V4L2 frame per interval, maps average brightness to an approximate lux value, and writes it atomically to a file. The helper is intended for IR or grayscale camera nodes such as `/dev/video2`; use `v4l2-ctl --list-formats-ext -d /dev/video2` to confirm that the node supports `GREY`.

```kdl
spawn-sh-at-startup "niri-camera-lux --device /dev/video2 --output $XDG_RUNTIME_DIR/niri-ambient-lux"

night-light {
    adaptive {
        on
        sensor-path "$XDG_RUNTIME_DIR/niri-ambient-lux"
        backlight-name "intel_backlight"
    }
}
```

`sensor-path` and `backlight-path` expand a leading `$XDG_RUNTIME_DIR` or `${XDG_RUNTIME_DIR}`.

If the camera is too sensitive or too dim, tune `niri-camera-lux --max-lux`. This value is the approximate lux reported for a pure-white frame.

### `overview`

<sup>Since: 25.05</sup>

Settings for the [Overview](./Overview.md).

#### `zoom`

Control how much the workspaces zoom out in the overview.
`zoom` ranges from 0 to 0.75 where lower values make everything smaller.

```kdl
// Make workspaces four times smaller than normal in the overview.
overview {
    zoom 0.25
}
```

#### `backdrop-color`

Set the backdrop color behind workspaces in the overview.
The backdrop is also visible between workspaces when switching.

The alpha channel for this color will be ignored.

```kdl
// Make the backdrop light.
overview {
    backdrop-color "#777777"
}
```

You can also set the color per-output [in the output config](./Configuration:-Outputs.md#backdrop-color).

#### `workspace-shadow`

Control the shadow behind workspaces visible in the overview.

Settings here mirror the normal [`shadow` config in the layout section](./Configuration:-Layout.md#shadow), so check the documentation there.

Workspace shadows are configured for a workspace size normalized to 1080 pixels tall, then zoomed out together with the workspace.
Practically, this means that you'll want bigger spread, offset, and softness compared to window shadows.

```kdl
// Disable workspace shadows in the overview.
overview {
    workspace-shadow {
        off
    }
}
```

### `xwayland-satellite`

<sup>Since: 25.08</sup>

Settings for integration with [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite).

When a recent enough xwayland-satellite is detected, niri will create the X11 sockets and set `DISPLAY`, then automatically spawn `xwayland-satellite` when an X11 client tries to connect.
If Xwayland dies, niri will keep watching the X11 socket and restart `xwayland-satellite` as needed.
This is very similar to how built-in Xwayland works in other compositors.

`off` disables the integration: niri won't create an X11 socket and won't set the `DISPLAY` environment variable.

`path` sets the path to the `xwayland-satellite` binary.
By default, it's just `xwayland-satellite`, so it's looked up like any other non-absolute program name.

```kdl
// Use a custom build of xwayland-satellite.
xwayland-satellite {
    path "~/source/rs/xwayland-satellite/target/release/xwayland-satellite"
}
```

### `clipboard`

<sup>Since: 25.02</sup>

Clipboard settings.

Set the `disable-primary` flag to disable the primary clipboard (middle-click paste).
Toggling this flag will only apply to applications started afterward.

```kdl
clipboard {
    disable-primary
}
```

### `hotkey-overlay`

Settings for the "Important Hotkeys" overlay.

#### `skip-at-startup`

Set the `skip-at-startup` flag if you don't want to see the hotkey help at niri startup.

```kdl
hotkey-overlay {
    skip-at-startup
}
```

#### `hide-not-bound`

<sup>Since: 25.08</sup>

By default, niri will show the most important actions even if they aren't bound to any key, to prevent confusion.
Set the `hide-not-bound` flag if you want to hide all actions not bound to any key.

```kdl
hotkey-overlay {
    hide-not-bound
}
```

You can customize which binds the hotkey overlay shows using the [`hotkey-overlay-title` property](./Configuration:-Key-Bindings.md#custom-hotkey-overlay-titles).

### `config-notification`

<sup>Since: 25.08</sup>

Settings for the config created/failed notification.

Set the `disable-failed` flag to disable the "Failed to parse the config file" notification.
For example, if you have a custom one.

```kdl
config-notification {
    disable-failed
}
```

### `blur`

<sup>Since: 26.04</sup>

Blur configuration that affects all background blur.

See the [window effects page](./Window-Effects.md) for an overview of background effects.

```kdl
// These are the default values:
blur {
    // off
    passes 3
    offset 3
    noise 0.02
    saturation 1.5
}
```

#### `off`

By default, blur is available on request by a window or layer surface (via the `ext-background-effect` protocol).
You can also enable it manually with the `blur true` background effect [window](./Configuration:-Window-Rules.md#background-effect) or [layer](./Configuration:-Layer-Rules.md#background-effect) rule.

Setting the `off` flag will disable all blur, both requested by the window, and configured in window rules.

```kdl
blur {
    off
}
```

#### `passes` and `offset`

`passes` controls the number of downsample/upsample passes for dual kawase blur.
More passes produce a larger, smoother blur, but cost more GPU resources.

`offset` is the pixel offset multiplier for each pass.
Offset `1` is the original dual kawase blur.
Larger values produce a smoother blur, at no additional GPU cost.

However, setting `offset` too big will produce visual artifacts.
You will need to increase `passes` to be able to use a bigger `offset` without artifacts.

When configuring blur, try increasing `offset` first (since it doesn't cause any extra GPU load) until you start getting artifacts.
Then, if you still need smoother blur, increase `passes` by 1.
Keep doing this until you get the desired visuals. 

```kdl
blur {
    passes 3
    offset 3.0
}
```

#### `noise`

Amount of noise to add on top of the blur.

This is helpful to reduce color banding artifacts.

```kdl
blur {
    noise 0.02
}
```

#### `saturation`

Color saturation applied to the blurred background.

Values above `1` increase saturation; values below `1` reduce it.

```kdl
blur {
    saturation 1.5
}
```
