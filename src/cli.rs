use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use niri_ipc::{Action, OutputAction};

use crate::utils::version;

#[derive(Parser)]
#[command(author, version = version(), about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
#[command(subcommand_value_name = "SUBCOMMAND")]
#[command(subcommand_help_heading = "Subcommands")]
pub struct Cli {
    /// Path to config file (default: `$XDG_CONFIG_HOME/niri/config.kdl`).
    ///
    /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
    /// command line argument takes precedence.
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Import environment globally to systemd and D-Bus, run D-Bus services.
    ///
    /// Set this flag in a systemd service started by your display manager, or when running
    /// manually as your main compositor instance. Do not set when running as a nested window, or
    /// on a TTY as your non-main compositor instance, to avoid messing up the global environment.
    #[arg(long)]
    pub session: bool,
    /// Command to run upon compositor startup.
    #[arg(last = true)]
    pub command: Vec<OsString>,

    #[command(subcommand)]
    pub subcommand: Option<Sub>,
}

#[derive(Subcommand)]
pub enum Sub {
    /// Communicate with the running niri instance.
    Msg {
        #[command(subcommand)]
        msg: Msg,
        /// Format output as JSON.
        #[arg(short, long)]
        json: bool,
    },
    /// Validate the config file.
    Validate {
        /// Path to config file (default: `$XDG_CONFIG_HOME/niri/config.kdl`).
        ///
        /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
        /// command line argument takes precedence.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Cause a panic to check if the backtraces are good.
    Panic,
    /// Generate shell completions.
    Completions { shell: CompletionShell },
}

#[derive(Subcommand)]
pub enum Msg {
    /// List connected outputs.
    Outputs,
    /// List workspaces.
    Workspaces,
    /// List open windows.
    Windows,
    /// List open layer-shell surfaces.
    Layers,
    /// Get the configured keyboard layouts.
    KeyboardLayouts,
    /// Print information about the focused output.
    FocusedOutput,
    /// Print information about the focused window.
    FocusedWindow,
    /// Pick a window with the mouse and print information about it.
    PickWindow,
    /// Pick a color from the screen with the mouse.
    PickColor,
    /// Perform an action.
    Action {
        #[command(subcommand)]
        action: Action,
    },
    /// Change output configuration temporarily.
    ///
    /// The configuration is changed temporarily and not saved into the config file. If the output
    /// configuration subsequently changes in the config file, these temporary changes will be
    /// forgotten.
    Output {
        /// Output name.
        ///
        /// Run `niri msg outputs` to see the output names.
        #[arg()]
        output: String,
        /// Configuration to apply.
        #[command(subcommand)]
        action: OutputAction,
    },
    /// Start continuously receiving events from the compositor.
    EventStream,
    /// Print the version of the running niri instance.
    Version,
    /// Request an error from the running niri instance.
    RequestError,
    /// Print the overview state.
    OverviewState,
    /// List screencasts.
    Casts,
    /// Render a window alone and write it to a PNG file.
    ///
    /// Works for tiled, offscreen and covered windows. Does not touch the clipboard. Returns once
    /// the file has been fully written.
    CaptureWindow {
        /// Path to write the PNG to.
        #[arg(long)]
        path: String,
        /// Id of the window to capture.
        ///
        /// If not set, uses the focused window.
        #[arg(long)]
        id: Option<u64>,
        /// Include the mouse pointer if it is over the window.
        #[arg(long)]
        show_pointer: bool,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
    Nushell,
}

impl TryFrom<CompletionShell> for Shell {
    type Error = &'static str;

    fn try_from(shell: CompletionShell) -> Result<Self, Self::Error> {
        match shell {
            CompletionShell::Bash => Ok(Shell::Bash),
            CompletionShell::Elvish => Ok(Shell::Elvish),
            CompletionShell::Fish => Ok(Shell::Fish),
            CompletionShell::PowerShell => Ok(Shell::PowerShell),
            CompletionShell::Zsh => Ok(Shell::Zsh),
            CompletionShell::Nushell => Err("Nushell should be handled separately"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_suspend_action() {
        let cli = Cli::parse_from(["niri", "msg", "action", "suspend"]);

        match cli.subcommand {
            Some(Sub::Msg {
                msg: Msg::Action {
                    action: Action::Suspend {},
                },
                json: false,
            }) => (),
            _ => panic!("expected suspend action"),
        }
    }

    #[test]
    fn parses_capture_window() {
        let cli = Cli::parse_from([
            "niri",
            "msg",
            "capture-window",
            "--path",
            "/tmp/w.png",
            "--id",
            "7",
            "--show-pointer",
        ]);

        match cli.subcommand {
            Some(Sub::Msg {
                msg:
                    Msg::CaptureWindow {
                        path,
                        id: Some(7),
                        show_pointer: true,
                    },
                json: false,
            }) => assert_eq!(path, "/tmp/w.png"),
            _ => panic!("expected capture-window"),
        }

        let cli = Cli::parse_from(["niri", "msg", "capture-window", "--path", "/tmp/w.png"]);
        assert!(matches!(
            cli.subcommand,
            Some(Sub::Msg {
                msg: Msg::CaptureWindow {
                    id: None,
                    show_pointer: false,
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn parses_screenshot_copy_to_clipboard() {
        let cli = Cli::parse_from([
            "niri",
            "msg",
            "action",
            "screenshot-window",
            "--copy-to-clipboard=false",
        ]);
        assert!(matches!(
            cli.subcommand,
            Some(Sub::Msg {
                msg: Msg::Action {
                    action: Action::ScreenshotWindow {
                        copy_to_clipboard: false,
                        ..
                    },
                },
                ..
            })
        ));

        let cli = Cli::parse_from(["niri", "msg", "action", "screenshot-screen"]);
        assert!(matches!(
            cli.subcommand,
            Some(Sub::Msg {
                msg: Msg::Action {
                    action: Action::ScreenshotScreen {
                        copy_to_clipboard: true,
                        ..
                    },
                },
                ..
            })
        ));

        let cli = Cli::parse_from(["niri", "msg", "action", "screenshot", "-c", "false"]);
        assert!(matches!(
            cli.subcommand,
            Some(Sub::Msg {
                msg: Msg::Action {
                    action: Action::Screenshot {
                        copy_to_clipboard: false,
                        ..
                    },
                },
                ..
            })
        ));
    }
}
