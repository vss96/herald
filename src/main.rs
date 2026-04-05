mod app;
mod events;
mod session;
mod tmux;
mod tui;

use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEvent};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::prelude::*;
use tokio::sync::mpsc;

use app::App;
use events::hook_listener::HookListener;
use events::types::HookEvent;

/// Events the main loop processes.
enum AppEvent {
    Key(KeyEvent),
    Hook(HookEvent),
    Tick,
}

/// Determine the runtime directory for sockets and buffers.
fn runtime_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("herald")
    } else {
        let uid = unsafe { libc::getuid() };
        std::env::temp_dir().join(format!("herald-{}", uid))
    }
}

/// Ensure runtime directory exists with correct permissions.
fn ensure_runtime_dir(dir: &PathBuf) -> Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).context("creating runtime directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .context("setting runtime directory permissions")?;
        }
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dir).context("reading runtime dir metadata")?;
            let uid = unsafe { libc::getuid() };
            if meta.uid() != uid {
                anyhow::bail!(
                    "runtime directory {} is owned by uid {}, expected {}",
                    dir.display(),
                    meta.uid(),
                    uid
                );
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o700 {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                    .context("tightening runtime directory permissions to 0700")?;
                tracing::warn!(
                    "runtime directory {} had mode {:o}, tightened to 0700",
                    dir.display(),
                    mode
                );
            }
        }
    }

    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }

    Ok(())
}

/// Install the hook script to the runtime directory.
fn install_hook_script(runtime_dir: &PathBuf) -> Result<PathBuf> {
    let hook_path = runtime_dir.join("herald-hook.py");
    let script = include_str!("../scripts/herald-hook.py");
    std::fs::write(&hook_path, script).context("writing hook script")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o700))
            .context("setting hook script permissions")?;
    }
    Ok(hook_path)
}

/// Spawn a thread that reads crossterm keyboard events and sends them to a channel.
fn spawn_keyboard_reader(tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            // Block on crossterm — this is intentional, it's on its own thread
            if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if tx.send(AppEvent::Key(key)).is_err() {
                        break; // Channel closed, app is shutting down
                    }
                }
            }
        }
    });
}

/// Spawn a hook listener for a session, forwarding events to the channel.
/// Drains any buffered events first (from before the socket was listening).
pub fn spawn_hook_listener(
    runtime_dir: &PathBuf,
    session_id: &str,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    let listener = HookListener::new(runtime_dir, session_id);

    let (hook_tx, mut hook_rx) = mpsc::channel::<HookEvent>(64);

    // Spawn: drain buffer first, then start socket listener
    let listener_socket = listener.socket_path().to_path_buf();
    let drain_tx = tx.clone();
    tokio::spawn(async move {
        // Drain buffered events before accepting live traffic
        match listener.drain_buffer().await {
            Ok(events) => {
                let count = events.len();
                if count > 0 {
                    tracing::info!(count, "drained buffered events for session");
                }
                for event in events {
                    let _ = drain_tx.send(AppEvent::Hook(event));
                }
            }
            Err(e) => {
                tracing::warn!("failed to drain buffer: {}", e);
            }
        }

        // Now start the live socket listener
        if let Err(e) = listener.listen(hook_tx).await {
            tracing::error!(socket = %listener_socket.display(), "hook listener error: {}", e);
        }
    });

    // Forwarder: bridge hook channel into the main AppEvent channel
    tokio::spawn(async move {
        while let Some(event) = hook_rx.recv().await {
            if tx.send(AppEvent::Hook(event)).is_err() {
                break;
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to per-run timestamped file
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("herald")
        .join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
    let log_filename = format!("herald-{}.log", timestamp);
    let log_path = log_dir.join(&log_filename);

    let latest_link = log_dir.join("latest.log");
    let _ = std::fs::remove_file(&latest_link);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&log_path, &latest_link);

    let log_file = std::fs::File::create(&log_path).ok();
    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_target(true)
            .with_level(true)
            .with_timer(tracing_subscriber::fmt::time::uptime())
            .init();
    }

    tracing::info!("herald starting, log: {}", log_path.display());
    tracing::info!("pid: {}, uid: {}", std::process::id(), unsafe { libc::getuid() });

    // Setup runtime directory
    let rt_dir = runtime_dir();
    tracing::info!("runtime dir: {}", rt_dir.display());
    ensure_runtime_dir(&rt_dir)?;
    install_hook_script(&rt_dir)?;

    eprintln!("herald: log file at {}", log_path.display());

    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let mut app = App::new(rt_dir.clone(), size.width, size.height);

    // Try to discover existing sessions
    match app.session_manager.ensure_tmux_session().await {
        Ok(()) => {
            if let Ok(discovered) = app.session_manager.discover_existing().await {
                for sid in &discovered {
                    tracing::info!(session_id = %sid, "discovered existing session");
                }
                if !discovered.is_empty() {
                    app.active_session_id = Some(discovered[0].clone());
                }
            }
        }
        Err(e) => {
            tracing::warn!("tmux not available: {}", e);
        }
    }

    // Create the receiver for app — replace the dummy one
    let (final_tx, final_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Re-create keyboard reader and hook listeners with the final channel
    // (The original event_tx was used for discovery; now wire up the real one)
    spawn_keyboard_reader(final_tx.clone());
    // Re-spawn listeners for discovered sessions on the final channel
    for sid in app.session_manager.sessions().map(|s| s.id.clone()).collect::<Vec<_>>() {
        spawn_hook_listener(&rt_dir, &sid, final_tx.clone());
    }

    app.event_tx = Some(final_tx);
    app.event_rx = final_rx;

    // Main event loop
    let result = run_loop(&mut terminal, &mut app).await;

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(150));
    // Periodically drain buffer files as a fallback (every 2 seconds)
    let mut drain_interval = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        // Render every tick
        terminal.draw(|frame| {
            app.render(frame.area(), frame.buffer_mut());
        })?;

        // Wait for the next event from any source
        tokio::select! {
            // Keyboard or hook event from the unified channel
            event = app.event_rx.recv() => {
                match event {
                    Some(AppEvent::Key(key)) => {
                        app.handle_key(key).await;
                    }
                    Some(AppEvent::Hook(hook_event)) => {
                        app.handle_hook_event(hook_event);
                    }
                    Some(AppEvent::Tick) => {}
                    None => break, // All senders dropped
                }
            }
            // Tick: refresh active terminal from tmux capture-pane
            _ = tick_interval.tick() => {
                app.refresh_active_terminal().await;
            }
            // Periodic buffer drain: catches events missed by socket delivery
            _ = drain_interval.tick() => {
                app.drain_all_buffers().await;
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
