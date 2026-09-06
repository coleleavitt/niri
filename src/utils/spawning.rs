use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::{io, thread};

use atomic::Atomic;
use libc::{getrlimit, rlim_t, rlimit, setrlimit, RLIMIT_NOFILE};
use niri_config::Environment;
use smithay::wayland::xdg_activation::XdgActivationToken;

use crate::utils::expand_home;

pub static REMOVE_ENV_RUST_BACKTRACE: AtomicBool = AtomicBool::new(false);
pub static REMOVE_ENV_RUST_LIB_BACKTRACE: AtomicBool = AtomicBool::new(false);
pub static CHILD_ENV: RwLock<Environment> = RwLock::new(Environment(Vec::new()));
pub static CHILD_DISPLAY: RwLock<Option<String>> = RwLock::new(None);

static ORIGINAL_NOFILE_RLIMIT_CUR: Atomic<rlim_t> = Atomic::new(0);
static ORIGINAL_NOFILE_RLIMIT_MAX: Atomic<rlim_t> = Atomic::new(0);

/// Increases the nofile rlimit to the maximum and stores the original value.
pub fn store_and_increase_nofile_rlimit() {
    let mut rlim = rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(RLIMIT_NOFILE, &mut rlim) } != 0 {
        let err = io::Error::last_os_error();
        warn!("error getting nofile rlimit: {err:?}");
        return;
    }

    ORIGINAL_NOFILE_RLIMIT_CUR.store(rlim.rlim_cur, Ordering::SeqCst);
    ORIGINAL_NOFILE_RLIMIT_MAX.store(rlim.rlim_max, Ordering::SeqCst);

    trace!(
        "changing nofile rlimit from {} to {}",
        rlim.rlim_cur,
        rlim.rlim_max
    );
    rlim.rlim_cur = rlim.rlim_max;

    if unsafe { setrlimit(RLIMIT_NOFILE, &rlim) } != 0 {
        let err = io::Error::last_os_error();
        warn!("error setting nofile rlimit: {err:?}");
    }
}

/// Restores the original nofile rlimit.
pub fn restore_nofile_rlimit() {
    let rlim_cur = ORIGINAL_NOFILE_RLIMIT_CUR.load(Ordering::SeqCst);
    let rlim_max = ORIGINAL_NOFILE_RLIMIT_MAX.load(Ordering::SeqCst);

    if rlim_cur == 0 {
        return;
    }

    let rlim = rlimit { rlim_cur, rlim_max };
    unsafe { setrlimit(RLIMIT_NOFILE, &rlim) };
}

/// Spawns the command to run independently of the compositor.
pub fn spawn<T: AsRef<OsStr> + Send + 'static>(command: Vec<T>, token: Option<XdgActivationToken>) {
    let _span = tracy_client::span!();

    if command.is_empty() {
        return;
    }

    // Spawning and waiting takes some milliseconds, so do it in a thread.
    let res = thread::Builder::new()
        .name("Command Spawner".to_owned())
        .spawn(move || {
            let (command, args) = command.split_first().unwrap();
            spawn_sync(command, args, token);
        });

    if let Err(err) = res {
        warn!("error spawning a thread to spawn the command: {err:?}");
    }
}

/// Spawns the command through the shell.
///
/// We hardcode `sh -c`, consistent with other compositors:
///
/// - https://github.com/swaywm/sway/blob/b3dcde8d69c3f1304b076968a7a64f54d0c958be/sway/commands/exec_always.c#L64
/// - https://github.com/hyprwm/Hyprland/blob/1ac1ff457ab8ef1ae6a8f2ab17ee7965adfa729f/src/managers/KeybindManager.cpp#L987
pub fn spawn_sh(command: String, token: Option<XdgActivationToken>) {
    spawn(vec![String::from("sh"), String::from("-c"), command], token);
}

fn spawn_sync(
    command: impl AsRef<OsStr>,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    token: Option<XdgActivationToken>,
) {
    let _span = tracy_client::span!();

    let mut command = command.as_ref();

    // Expand `~` at the start.
    let expanded = expand_home(Path::new(command));
    match &expanded {
        Ok(Some(expanded)) => command = expanded.as_ref(),
        Ok(None) => (),
        Err(err) => {
            warn!("error expanding ~: {err:?}");
        }
    }

    let mut process = Command::new(command);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Remove RUST_BACKTRACE and RUST_LIB_BACKTRACE from the environment if needed.
    if REMOVE_ENV_RUST_BACKTRACE.load(Ordering::Relaxed) {
        process.env_remove("RUST_BACKTRACE");
    }
    if REMOVE_ENV_RUST_LIB_BACKTRACE.load(Ordering::Relaxed) {
        process.env_remove("RUST_LIB_BACKTRACE");
    }

    // Remove the systemd NOTIFY_SOCKET variable.
    process.env_remove("NOTIFY_SOCKET");

    // Set DISPLAY if needed.
    let display = CHILD_DISPLAY.read().unwrap();
    if let Some(display) = &*display {
        process.env("DISPLAY", display);
    } else {
        process.env_remove("DISPLAY");
    }

    // Set configured environment.
    let env = CHILD_ENV.read().unwrap();
    for var in &env.0 {
        if let Some(value) = &var.value {
            process.env(&var.name, value);
        } else {
            process.env_remove(&var.name);
        }
    }
    drop(env);

    if let Some(token) = token.as_ref() {
        process.env("XDG_ACTIVATION_TOKEN", token.as_str());
        process.env("DESKTOP_STARTUP_ID", token.as_str());
    }

    unsafe { process.pre_exec(crate::utils::signals::unblock_all) };

    let Some(mut child) = do_spawn(command, process) else {
        return;
    };

    match child.wait() {
        Ok(status) => {
            if !status.success() {
                warn!("child did not exit successfully: {status:?}");
            }
        }
        Err(err) => {
            warn!("error waiting for child: {err:?}");
        }
    }
}

#[cfg(not(feature = "systemd"))]
fn do_spawn(command: &OsStr, mut process: Command) -> Option<Child> {
    unsafe {
        // Double-fork to avoid having to waitpid the child.
        process.pre_exec(move || {
            match libc::fork() {
                -1 => return Err(io::Error::last_os_error()),
                0 => (),
                _ => libc::_exit(0),
            }

            restore_nofile_rlimit();

            Ok(())
        });
    }

    let child = match process.spawn() {
        Ok(child) => child,
        Err(err) => {
            warn!("error spawning {command:?}: {err:?}");
            return None;
        }
    };

    Some(child)
}

#[cfg(feature = "systemd")]
use systemd::do_spawn;

#[cfg(feature = "systemd")]
mod systemd {
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

    use smithay::reexports::rustix;
    use smithay::reexports::rustix::io::{close, read, retry_on_intr, write};
    use smithay::reexports::rustix::pipe::{pipe_with, PipeFlags};

    use super::*;

    pub fn do_spawn(command: &OsStr, mut process: Command) -> Option<Child> {
        #[cfg(target_env = "gnu")]
        use libc::close_range;
        #[cfg(target_os = "openbsd")]
        use libc::closefrom;

        #[cfg(not(target_env = "gnu"))] // musl
        pub fn close_range(first: libc::c_uint, last: libc::c_uint, flags: libc::c_uint) -> i64 {
            unsafe {
                libc::syscall(
                    libc::SYS_close_range,
                    first as usize,
                    last as usize,
                    flags as usize,
                )
            }
        }

        // When running as a systemd session, we want to put children into their own transient
        // scopes in order to separate them from the niri process. This is helpful for
        // example to prevent the OOM killer from taking down niri together with a
        // misbehaving client.
        //
        // Putting a child into a scope is done by calling systemd's StartTransientUnit D-Bus method
        // with a PID. Unfortunately, there seems to be a race in systemd where if the child exits
        // at just the right time, the transient unit will be created but empty, so it will
        // linger around forever.
        //
        // To prevent this, we'll use our double-fork (done for a separate reason) to help. In our
        // intermediate child we will send back the grandchild PID, and in niri we will create a
        // transient scope with both our intermediate child and the grandchild PIDs set. Only then
        // we will signal our intermediate child to exit. This way, even if the grandchild
        // exits quickly, a non-empty scope will be created (with just our intermediate
        // child), then cleaned up when our intermediate child exits.

        // Make a pipe to receive the grandchild PID.

        let (pipe_pid_read, pipe_pid_write) = pipe_with(PipeFlags::CLOEXEC)
            .map_err(|err| {
                warn!("error creating a pipe to transfer child PID: {err:?}");
            })
            .ok()
            .unzip();
        // Make a pipe to wait in the intermediate child.
        let (pipe_wait_read, pipe_wait_write) = pipe_with(PipeFlags::CLOEXEC)
            .map_err(|err| {
                warn!("error creating a pipe for child to wait on: {err:?}");
            })
            .ok()
            .unzip();

        unsafe {
            // The fds will be duplicated after a fork and closed on exec or exit automatically. Get
            // the raw fd inside so that it's not closed any extra times.
            let mut pipe_pid_read_fd = pipe_pid_read.as_ref().map(|fd| fd.as_raw_fd());
            let mut pipe_pid_write_fd = pipe_pid_write.as_ref().map(|fd| fd.as_raw_fd());
            let mut pipe_wait_read_fd = pipe_wait_read.as_ref().map(|fd| fd.as_raw_fd());
            let mut pipe_wait_write_fd = pipe_wait_write.as_ref().map(|fd| fd.as_raw_fd());

            // Double-fork to avoid having to waitpid the child.
            process.pre_exec(move || {
                // Close FDs that we don't need. Especially important for the write ones to unblock
                // the readers.
                if let Some(fd) = pipe_pid_read_fd.take() {
                    close(fd);
                }
                if let Some(fd) = pipe_wait_write_fd.take() {
                    close(fd);
                }

                // Convert the FDs to OwnedFd, which will close them in all of our fork paths.
                let pipe_pid_write = pipe_pid_write_fd.take().map(|fd| OwnedFd::from_raw_fd(fd));
                let pipe_wait_read = pipe_wait_read_fd.take().map(|fd| OwnedFd::from_raw_fd(fd));

                match libc::fork() {
                    -1 => return Err(io::Error::last_os_error()),
                    0 => (),
                    grandchild_pid => {
                        // Send back the PID.
                        if let Some(pipe) = pipe_pid_write {
                            let _ = write_all(pipe, &grandchild_pid.to_ne_bytes());
                        }

                        // Wait until the parent signals us to exit.
                        if let Some(pipe) = pipe_wait_read {
                            // We're going to exit afterwards. Close all other FDs to allow
                            // Command::spawn() to return in the parent process.
                            #[cfg(not(target_os = "openbsd"))]
                            {
                                let raw = pipe.as_raw_fd() as u32;
                                let _ = close_range(0, raw - 1, 0);
                                let _ = close_range(raw + 1, !0, 0);
                            }
                            #[cfg(target_os = "openbsd")]
                            {
                                let raw = pipe.as_raw_fd();
                                for fd in 0..raw {
                                    close(fd);
                                }
                                closefrom(raw + 1);
                            }

                            wait_for_parent(pipe, grandchild_pid);
                        }

                        libc::_exit(0)
                    }
                }

                restore_nofile_rlimit();

                Ok(())
            });
        }

        let child = match process.spawn() {
            Ok(child) => child,
            Err(err) => {
                warn!("error spawning {command:?}: {err:?}");
                return None;
            }
        };

        drop(pipe_pid_write);
        drop(pipe_wait_read);

        // Tell the intermediate child that exec() succeeded, so from now on it must stay alive
        // until we're done with the systemd scope even if the grandchild exits quickly.
        if let Some(pipe) = &pipe_wait_write {
            let _ = write_all(pipe, &[EXEC_OK]);
        }

        // Wait for the grandchild PID.
        if let Some(pipe) = pipe_pid_read {
            let mut buf = [0; 4];
            match read_all(pipe, &mut buf) {
                Ok(()) => {
                    let pid = i32::from_ne_bytes(buf);
                    trace!("spawned PID: {pid}");

                    // Start a systemd scope for the grandchild.
                    if let Err(err) = start_systemd_scope(command, child.id(), pid as u32) {
                        trace!("error starting systemd scope for spawned command: {err:?}");
                    }
                }
                Err(err) => {
                    warn!("error reading child PID: {err:?}");
                }
            }
        }

        // Signal the intermediate child to exit now that we're done trying to creating a systemd
        // scope.
        trace!("signaling child to exit");
        drop(pipe_wait_write);

        Some(child)
    }

    /// Byte the parent writes to the wait pipe once `Command::spawn()` has returned successfully.
    const EXEC_OK: u8 = 1;

    /// Blocks the intermediate child until the parent is done with it.
    ///
    /// The parent holds the write end of `pipe` across `Command::spawn()`. If the grandchild
    /// fails to exec (e.g. the binary is missing), Rust's `spawn()` calls `wait()` on *us*
    /// before returning the error to the parent, while we would be sitting here waiting for the
    /// parent to drop the pipe: a deadlock that leaks a thread and a zombie per failed spawn, and
    /// swallows the error message entirely.
    ///
    /// So until the parent confirms that exec succeeded ([`EXEC_OK`]), we also watch the
    /// grandchild through a pidfd and exit as soon as it dies. After the confirmation we ignore
    /// the grandchild and wait for the pipe to close, which is what keeps the systemd scope
    /// non-empty for short-lived commands.
    fn wait_for_parent(pipe: OwnedFd, grandchild_pid: libc::pid_t) {
        #[cfg(target_os = "linux")]
        {
            let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, grandchild_pid, 0) };
            if pidfd >= 0 {
                let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as _) };
                let mut exec_confirmed = false;

                loop {
                    let mut fds = [
                        libc::pollfd {
                            fd: pipe.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: pidfd.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    let nfds = if exec_confirmed { 1 } else { 2 };

                    let rc = unsafe { libc::poll(fds.as_mut_ptr(), nfds, -1) };
                    if rc < 0 {
                        if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        return;
                    }

                    if fds[0].revents != 0 {
                        let mut byte = [0u8; 1];
                        match retry_on_intr(|| read(&pipe, &mut byte)) {
                            Ok(1) if byte[0] == EXEC_OK => exec_confirmed = true,
                            // EOF or error: the parent is done with us.
                            _ => return,
                        }
                    }

                    if !exec_confirmed && fds[1].revents != 0 {
                        // The grandchild died before exec() was confirmed: most likely exec
                        // failed, and the parent is blocked in wait() for us. Let it go.
                        return;
                    }
                }
            }
        }

        // No pidfd support: fall back to plain blocking. Consume the EXEC_OK byte (if any) and
        // then wait for EOF.
        let mut byte = [0u8; 1];
        loop {
            match retry_on_intr(|| read(&pipe, &mut byte)) {
                Ok(1) => continue,
                _ => return,
            }
        }
    }

    fn write_all(fd: impl AsFd, buf: &[u8]) -> rustix::io::Result<()> {
        let mut written = 0;
        loop {
            let n = retry_on_intr(|| write(&fd, &buf[written..]))?;
            if n == 0 {
                return Err(rustix::io::Errno::CANCELED);
            }

            written += n;
            if written == buf.len() {
                return Ok(());
            }
        }
    }

    fn read_all(fd: impl AsFd, buf: &mut [u8]) -> rustix::io::Result<()> {
        let mut start = 0;
        loop {
            let n = retry_on_intr(|| read(&fd, &mut buf[start..]))?;
            if n == 0 {
                return Err(rustix::io::Errno::CANCELED);
            }

            start += n;
            if start == buf.len() {
                return Ok(());
            }
        }
    }

    /// Puts a (newly spawned) pid into a transient systemd scope.
    ///
    /// This separates the pid from the compositor scope, which for example prevents the OOM killer
    /// from bringing down the compositor together with a misbehaving client.
    fn start_systemd_scope(
        name: &OsStr,
        intermediate_pid: u32,
        child_pid: u32,
    ) -> anyhow::Result<()> {
        use std::fmt::Write as _;
        use std::os::unix::ffi::OsStrExt;
        use std::sync::OnceLock;

        use anyhow::Context;
        use zbus::zvariant::{OwnedObjectPath, Value};

        use crate::utils::IS_SYSTEMD_SERVICE;

        // We only start transient scopes if we're a systemd service ourselves.
        if !IS_SYSTEMD_SERVICE.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _span = tracy_client::span!();

        // Extract the basename.
        let name = Path::new(name).file_name().unwrap_or(name);

        let mut scope_name = String::from("app-niri-");

        // Escape for systemd similarly to libgnome-desktop, which says it had adapted this from
        // systemd source.
        for &c in name.as_bytes() {
            if c.is_ascii_alphanumeric() || matches!(c, b':' | b'_' | b'.') {
                scope_name.push(char::from(c));
            } else {
                let _ = write!(scope_name, "\\x{c:02x}");
            }
        }

        let _ = write!(scope_name, "-{child_pid}.scope");

        // Ask systemd to start a transient scope.
        static CONNECTION: OnceLock<zbus::Result<zbus::blocking::Connection>> = OnceLock::new();
        let conn = CONNECTION
            .get_or_init(zbus::blocking::Connection::session)
            .clone()
            .context("error connecting to session bus")?;

        let proxy = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .context("error creating a Proxy")?;

        let signals = proxy
            .receive_signal("JobRemoved")
            .context("error creating a signal iterator")?;

        let pids: &[_] = &[intermediate_pid, child_pid];
        let properties: &[_] = &[
            ("PIDs", Value::new(pids)),
            ("CollectMode", Value::new("inactive-or-failed")),
        ];
        let aux: &[(&str, &[(&str, Value)])] = &[];

        let job: OwnedObjectPath = proxy
            .call("StartTransientUnit", &(scope_name, "fail", properties, aux))
            .context("error calling StartTransientUnit")?;

        trace!("waiting for JobRemoved");
        for message in signals {
            let body = message.body();
            let body: (u32, OwnedObjectPath, &str, &str) =
                body.deserialize().context("error parsing signal")?;

            if body.1 == job {
                // Our transient unit had started, we're good to exit the intermediate child.
                break;
            }
        }

        Ok(())
    }
}
