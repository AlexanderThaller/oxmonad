//! oxmonad: an xmonad-style tiling window manager for river.
//!
//! oxmonad is river's window-manager client: river runs it (`river -c
//! oxmonad`) and it speaks `river-window-management-v1` over the Wayland
//! socket river hands it. See `docs/architecture.adoc`.

mod config_file;
mod control;
mod font_cache;
mod keyboard;
mod keys;
mod plan;
mod remote;
mod restart;
mod shell;
mod shm;
mod spawn;
mod state;
mod status;

use std::{
    env,
    io::{
        self,
        IsTerminal,
        Read,
    },
    os::unix::net::UnixStream,
    path::{
        Path,
        PathBuf,
    },
    process::ExitCode,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    time::Duration,
};

use calloop::{
    EventLoop,
    Interest,
    LoopHandle,
    Mode,
    PostAction,
    generic::Generic,
    timer::{
        TimeoutAction,
        Timer,
    },
};
use calloop_wayland_source::WaylandSource;
use river_protocols::{
    layer_shell::river_layer_shell_v1::RiverLayerShellV1,
    window_management::river_window_manager_v1::RiverWindowManagerV1,
    xkb_bindings::river_xkb_bindings_v1::RiverXkbBindingsV1,
};
use signal_hook::consts::{
    SIGCHLD,
    SIGINT,
    SIGTERM,
    SIGUSR1,
    SIGUSR2,
};
use tracing_subscriber::EnvFilter;
use wayland_client::{
    Connection,
    globals::registry_queue_init,
};

use crate::state::{
    Globals,
    State,
};

const USAGE: &str = "\
usage: oxmonad [--config PATH] [--check-config | --restart]

oxmonad is a window manager for the river compositor; river starts it:

    river -c oxmonad

Options:
    --config PATH    read PATH instead of $XDG_CONFIG_HOME/oxmonad/config.toml
    --check-config   validate the configuration and exit
    --restart        restart the running oxmonad of this session in place
                     (like Mod+q; sends it SIGUSR2)
    -h, --help       show this help

Environment:
    RUST_LOG         log filter (default: info)
    OXMONAD_CONTROL  listen for commands on this Unix socket path
    OXMONAD_RESTORE  (set by `restart`) the state file to restore
";

struct Args {
    config: Option<PathBuf>,
    check: bool,
    restart: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: config_file::default_path(),
        check: false,
        restart: false,
    };
    let mut iter = env::args_os().skip(1);
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--config" | "-c") => {
                let path = iter.next().ok_or("--config needs a path")?;
                args.config = Some(PathBuf::from(path));
            }
            Some("--check-config") => args.check = true,
            Some("--restart") => args.restart = true,
            Some("-h" | "--help") => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("unexpected argument {}", arg.to_string_lossy())),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("oxmonad: {err}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    if args.restart {
        return restart_running();
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .init();

    let config = match config_file::load(args.config.as_deref()) {
        Ok(config) => config,
        Err(err) if args.check => {
            eprintln!("oxmonad: {err}");
            return ExitCode::FAILURE;
        }
        Err(err) => {
            tracing::error!(%err, "using the default configuration");
            oxmonad_core::config::Config::defaults()
        }
    };
    if args.check {
        let unresolved: Vec<String> = config
            .keybindings
            .iter()
            .filter(|b| keys::resolve(&b.combo).is_none())
            .map(|b| b.combo.to_string())
            .collect();
        if !unresolved.is_empty() {
            eprintln!("oxmonad: unknown key names: {}", unresolved.join(", "));
            return ExitCode::FAILURE;
        }
        println!("configuration ok");
        return ExitCode::SUCCESS;
    }

    match run(config, args.config) {
        Ok(code) => code,
        Err(err) => {
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// `oxmonad --restart`: signals the running oxmonad of this session.
fn restart_running() -> ExitCode {
    match remote::restart_running() {
        Ok(pids) if pids.is_empty() => {
            eprintln!("oxmonad: no running oxmonad found in this session");
            ExitCode::FAILURE
        }
        Ok(pids) => {
            for pid in pids {
                println!("restarting oxmonad (pid {pid})");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("oxmonad: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    config: oxmonad_core::config::Config,
    config_path: Option<PathBuf>,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // Catch reload/restart requests from the very start: until the event
    // loop watches them (after connecting and loading fonts, which takes
    // a while), SIGUSR1/SIGUSR2 would otherwise kill oxmonad — e.g. an
    // `oxmonad --restart` right after a restart.
    let early_reload = Arc::new(AtomicBool::new(false));
    let early_restart = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGUSR1, Arc::clone(&early_reload))?;
    signal_hook::flag::register(SIGUSR2, Arc::clone(&early_restart))?;
    let restore = restart::take();
    let conn = Connection::connect_to_env().map_err(|err| {
        format!("cannot connect to river ({err}); run oxmonad with `river -c oxmonad`")
    })?;
    let (list, queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let wl_outputs = state::bind_wl_outputs(&list, &qh);
    let wm: RiverWindowManagerV1 = list.bind(&qh, 1..=5, ()).map_err(|_| {
        "river_window_manager_v1 is not available: oxmonad must be started by river 0.4+ (`river \
         -c oxmonad`)"
    })?;
    let xkb: Option<RiverXkbBindingsV1> = list.bind(&qh, 1..=3, ()).ok();
    let layer_shell: Option<RiverLayerShellV1> = list.bind(&qh, 1..=1, ()).ok();
    let compositor = list.bind(&qh, 4..=6, ())?;
    let shm = list.bind(&qh, 1..=1, ())?;
    let globals = Globals {
        list,
        wm,
        xkb,
        layer_shell,
        compositor,
        shm,
        wl_outputs,
    };

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|err| format!("cannot watch the Wayland socket: {}", err.error))?;
    let exe = restart::Exe::current()
        .inspect_err(|err| tracing::warn!(%err, "cannot locate the oxmonad binary; restart is off"))
        .ok();
    let mut state = State::new(
        handle.clone(),
        conn,
        qh,
        globals,
        (config, config_path),
        exe,
        restore,
    );

    watch_signal(&handle, &[SIGCHLD], State::reap_children)?;
    watch_signal(&handle, &[SIGTERM, SIGINT], State::stop)?;
    watch_signal(&handle, &[SIGUSR1], State::request_reload)?;
    watch_signal(&handle, &[SIGUSR2], State::request_restart)?;
    if early_reload.load(Ordering::Relaxed) {
        state.request_reload();
    }
    if early_restart.load(Ordering::Relaxed) {
        state.request_restart();
    }
    start_status_timers(&handle, &mut state)?;
    if let Some(path) = env::var_os(control::ENV).filter(|path| !path.is_empty()) {
        listen_control(&handle, Path::new(&path))?;
    }

    tracing::info!("oxmonad started");
    let signal = event_loop.get_signal();
    let mut code = ExitCode::SUCCESS;
    let result = event_loop.run(None, &mut state, |state| {
        state.flush();
        if let Some(exit) = state.exit_code() {
            code = exit;
            signal.stop();
        }
    });
    match result {
        Ok(()) => Ok(code),
        // river closes the connection when the session ends.
        Err(err) if state.is_quitting() => {
            tracing::debug!(%err, "river is gone");
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => Err(format!("lost the connection to river: {err}").into()),
    }
}

/// Calls `handler` whenever one of `signals` arrives (via a self-pipe, so
/// the signal mask children inherit stays untouched).
fn watch_signal(
    handle: &LoopHandle<'_, State>,
    signals: &[i32],
    handler: impl Fn(&mut State) + 'static,
) -> io::Result<()> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    writer.set_nonblocking(true)?;
    for &signal in signals {
        signal_hook::low_level::pipe::register(signal, writer.try_clone()?)?;
    }
    handle
        .insert_source(
            Generic::new(reader, Interest::READ, Mode::Level),
            move |_, reader, state| {
                let mut buffer = [0; 64];
                let mut stream: &UnixStream = reader.as_ref();
                while matches!(stream.read(&mut buffer), Ok(n) if n > 0) {}
                handler(state);
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(())
}

/// The clock (updated on minute boundaries) and battery (polled) timers.
fn start_status_timers(handle: &LoopHandle<'_, State>, state: &mut State) -> io::Result<()> {
    let clock = |state: &mut State| {
        let now = chrono::Local::now();
        state.set_status(|status| status.clock = status::clock_text(&now));
        status::until_next_minute(&now)
    };
    let battery = |state: &mut State| {
        let reading = status::read_battery(Path::new("/sys/class/power_supply"));
        state.set_status(|status| status.battery = reading);
    };
    let first = clock(state);
    battery(state);
    let insert = |timer: Timer, callback: Box<dyn FnMut(&mut State) -> Duration>| {
        let mut callback = callback;
        handle
            .insert_source(timer, move |_, (), state| {
                TimeoutAction::ToDuration(callback(state))
            })
            .map(|_| ())
            .map_err(|err| io::Error::other(err.to_string()))
    };
    insert(Timer::from_duration(first), Box::new(clock))?;
    insert(
        Timer::from_duration(status::BATTERY_INTERVAL),
        Box::new(move |state| {
            battery(state);
            status::BATTERY_INTERVAL
        }),
    )?;
    Ok(())
}

/// Serves the control socket (see [`control`]).
fn listen_control(handle: &LoopHandle<'_, State>, path: &Path) -> io::Result<()> {
    let listener = control::Listener::bind(path)?;
    tracing::info!(path = %path.display(), "control socket");
    let clients = handle.clone();
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, _| {
                while let Ok(client) = listener.accept() {
                    let fd = match client.socket().try_clone() {
                        Ok(fd) => fd,
                        Err(err) => {
                            tracing::warn!(%err, "control client");
                            continue;
                        }
                    };
                    let mut client = client;
                    let inserted = clients.insert_source(
                        Generic::new(fd, Interest::READ, Mode::Level),
                        move |_, _, state| {
                            let received = client.receive();
                            for line in received.lines {
                                match client.reply_handle() {
                                    Ok(reply) => state.control(&line, reply),
                                    Err(err) => tracing::warn!(%err, "control client"),
                                }
                            }
                            Ok(if received.closed {
                                PostAction::Remove
                            } else {
                                PostAction::Continue
                            })
                        },
                    );
                    if let Err(err) = inserted {
                        tracing::warn!(%err, "control client");
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(())
}
