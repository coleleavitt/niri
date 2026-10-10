You can communicate with the running niri instance over an IPC socket.
Check `niri msg --help` for available commands.

The `--json` flag prints the response in JSON, rather than formatted.
For example, `niri msg --json outputs`.

> [!TIP]
> If you're getting parsing errors from `niri msg` after upgrading niri, make sure that you've restarted niri itself.
> You might be trying to run a newer `niri msg` against an older `niri` compositor.

### Event Stream

<sup>Since: 0.1.9</sup>

While most niri IPC requests return a single response, the event stream request will make niri continuously stream events into the IPC connection until it is closed.
This is useful for implementing various bars and indicators that update as soon as something happens, without continuous polling.

The event stream IPC is designed to give you the complete current state up-front, then follow up with updates to that state.
This way, your state can never "desync" from niri, and you don't need to make any other IPC information requests.

Where reasonable, event stream state updates are atomic, though this is not always the case.
For example, a window may end up with a workspace id for a workspace that had already been removed.
This can happen if the corresponding workspaces-changed event arrives before the corresponding window-changed event.

To get a taste of the events, run `niri msg event-stream`.
Though, this is more of a debug function than anything.
You can get raw events from `niri msg --json event-stream`, or by connecting to the niri socket and requesting an event stream manually.

You can find the full list of events along with documentation [here](https://niri-wm.github.io/niri/niri_ipc/enum.Event.html).

### Window Geometry

<sup>Since: next release</sup>

`niri msg window-geometry` prints where a window is on screen right now, in the same global logical coordinates as the output positions from `niri msg outputs`.
Pass `--id` to pick a window by its id; otherwise niri uses the focused window.
Unlike the `layout` in `niri msg windows`, this works for tiled windows too, and it follows animations such as view scrolling and workspace switches.

```sh
$ niri msg --json window-geometry --id 12
{"id":12,"output":"DP-2","scale":1.5,"workspace_id":3,"is_floating":false,"on_screen":true,"tile_rect":{"x":2560.0,"y":16.0,"width":1052.0,"height":1408.0},"window_rect":{"x":2560.0,"y":16.0,"width":1052.0,"height":1408.0},"visible_rect":{"x":2560.0,"y":16.0,"width":1052.0,"height":1408.0},"fully_visible":true,"animating":false,"overview_open":false}
```

- `tile_rect` includes borders, `window_rect` is the window itself, and `visible_rect` is the part of `window_rect` inside the output.
- The rectangles are `null` when the window is not shown at normal scale: its workspace is not visible, it is a hidden tab, it is a floating window hidden behind a fullscreen window, or the overview is open or animating.
- `animating` is `true` while something that can move or resize the window is in progress. Query again once it becomes `false` before you rely on the position.

The geometry is computed on request rather than sent through the event stream, since it changes on every animation frame.

### Hit Testing

<sup>Since: next release</sup>

`niri msg window-at X Y` reports what a pointer click at a point would hit.
The point is in the global logical coordinate space, the same one as the logical positions in `niri msg outputs`.
The answer uses the same hit testing as real pointer input, so layer-shell surfaces on top, pop-ups, and the overview are taken into account.

```sh
$ niri msg --json window-at 120 80
{"output":"DP-1","window_id":12,"window_local":[104.0,64.0],"is_input":true,"layer":null}
```

- `window_id` is the topmost window under the point.
- `window_local` is the point relative to the top-left corner of the window geometry. It is only set when `is_input` is `true`.
- `is_input` is `false` when a click there would only activate the window, for example on its border or when the overview is open.
- `layer` is set when a layer-shell surface, such as a bar or a notification, is on top at that point.

### Programmatic Access

`niri msg --json` is a thin wrapper over writing and reading to a socket.
When implementing more complex scripts and modules, you're encouraged to access the socket directly.

Connect to the UNIX domain socket located at `$NIRI_SOCKET` in the filesystem.
Write your request encoded in JSON on a single line, followed by a newline character, or by flushing and shutting down the write end of the connection.
Read the reply as JSON, also on a single line.

You can use `socat` to test communicating with niri directly:

```sh
$ socat STDIO "$NIRI_SOCKET"
"FocusedWindow"
{"Ok":{"FocusedWindow":{"id":12,"title":"t socat STDIO /run/u ~","app_id":"Alacritty","workspace_id":6,"is_focused":true}}}
```

The reply is an `Ok` or an `Err` wrapping the same JSON object as you get from `niri msg --json`.

For more complex requests, you can use `socat` to find how `niri msg` formats them:

```sh
$ socat STDIO UNIX-LISTEN:temp.sock
# then, in a different terminal:
$ env NIRI_SOCKET=./temp.sock niri msg action focus-workspace 2
# then, look in the socat terminal:
{"Action":{"FocusWorkspace":{"reference":{"Index":2}}}}
```

You can find all available requests and response types in the [niri-ipc sub-crate documentation](https://niri-wm.github.io/niri/niri_ipc/).

### Capturing a Window

<sup>Since: next release</sup>

`niri msg capture-window --path /abs/path.png` renders one window alone and writes it to a PNG file.
It works for tiled, scrolled-out-of-view, and covered windows, and does not touch the clipboard.
The command returns only after the file has been fully written.

- `--id N` captures the window with that id instead of the focused window.
- `--show-pointer` includes the pointer if it is over the window.
- A relative `--path` is resolved against the working directory of `niri msg`. A raw `CaptureWindow` request over the socket needs an absolute path.

The JSON reply describes the image:

```sh
$ niri msg --json capture-window --path /tmp/window.png
{"id":12,"path":"/tmp/window.png","width_px":1210,"height_px":820,"scale":1.25,"window_offset_px":[5,10],"window_size_px":[1200,800]}
```

Sizes are in physical pixels at the output's `scale`.
Popups and client-side decorations can make the image larger than the window, so `window_offset_px` and `window_size_px` give the window geometry inside the image.

The `screenshot`, `screenshot-screen` and `screenshot-window` actions also take `--copy-to-clipboard=false` to keep the screenshot out of the clipboard.

### Backwards Compatibility

The JSON output *should* remain stable, as in:

- existing fields and enum variants should not be renamed
- non-optional existing fields should not be removed

However, new fields and enum variants will be added, so you should handle unknown fields or variants gracefully where reasonable.

The formatted/human-readable output (i.e. without `--json` flag) is **not** considered stable.
Please prefer the JSON output for scripts, since I reserve the right to make any changes to the human-readable output.

The `niri-ipc` sub-crate (like other niri sub-crates) is *not* API-stable in terms of the Rust semver; rather, it follows the version of niri itself.
In particular, new struct fields and enum variants will be added.
