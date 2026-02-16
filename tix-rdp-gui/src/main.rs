//! TIX RDP GUI Client — entry point.
//!
//! ```text
//! tix-rdp-gui                    Connect with defaults
//! tix-rdp-gui --config <path>   Use custom config TOML
//! tix-rdp-gui --gen-config      Dump default config and exit
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::Parser;
use tokio::net::UdpSocket;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use tix_core::rdp::client::ScreenClient;
use tix_core::rdp::transport::ScreenTransport;
use tix_core::rdp::types::PixelFormat;

use tix_rdp_gui::config::GuiConfig;
use tix_rdp_gui::connection::SlaveConnection;
use tix_rdp_gui::display::DisplayRenderer;
use tix_rdp_gui::input::{translate_event, InputAction};
use tix_rdp_gui::window::{NativeWindow, WindowEvent};

// ── CLI ──────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "tix-rdp-gui", about = "TIX RDP remote desktop viewer")]
struct Cli {
    /// Path to configuration TOML file.
    #[arg(short, long, default_value = "tix-rdp-gui.toml")]
    config: PathBuf,

    /// Slave address (overrides config). Example: 192.168.1.100:7332
    #[arg(short, long)]
    slave: Option<String>,

    /// Print the default configuration to stdout and exit.
    #[arg(long)]
    gen_config: bool,
}

// ── Main ─────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.gen_config {
        let text = toml::to_string_pretty(&GuiConfig::default())?;
        println!("{text}");
        return Ok(());
    }

    let mut config = GuiConfig::load(&cli.config);
    if let Some(addr) = cli.slave {
        config.network.slave_address = addr;
    }

    // Init tracing.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("tix-rdp-gui v{}", env!("CARGO_PKG_VERSION"));

    // ── 1. Create the window ────────────────────────────────────

    let window = NativeWindow::create(
        "TIX Remote Desktop",
        config.display.width,
        config.display.height,
    )?;
    let mut renderer = DisplayRenderer::new(
        window.hwnd(),
        config.display.width,
        config.display.height,
    );

    // ── 2. Connect to the slave ─────────────────────────────────

    // Bind a UDP socket for receiving screen frames.
    let udp = UdpSocket::bind("0.0.0.0:0").await?;
    let local_udp_port = udp.local_addr()?.port();

    let mut conn = SlaveConnection::connect(&config, local_udp_port).await?;
    let slave_screen_addr = conn.slave_screen_addr()?;
    info!("slave screen addr: {slave_screen_addr}");

    let transport = ScreenTransport::new(udp, slave_screen_addr);

    // ── 3. Start the RDP client ─────────────────────────────────

    let mut client = ScreenClient::new(transport, PixelFormat::Bgra8);
    let mut frame_rx = client.frame_receiver();
    let stats_rx = client.stats_receiver();
    let running = Arc::new(AtomicBool::new(true));

    let client_running = running.clone();
    let client_handle = tokio::spawn(async move {
        if let Err(e) = client.run().await {
            error!("RDP client error: {e}");
        }
        client_running.store(false, Ordering::SeqCst);
    });

    // ── 4. Event loop ───────────────────────────────────────────

    let mut remote_width = config.display.width;
    let mut remote_height = config.display.height;
    let mut win_width = config.display.width;
    let mut win_height = config.display.height;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Pump window messages.
        let events = window.poll_events();
        for ev in &events {
            match ev {
                WindowEvent::Close => {
                    running.store(false, Ordering::SeqCst);
                    break;
                }
                WindowEvent::Resize(w, h) => {
                    win_width = *w;
                    win_height = *h;
                    renderer.resize(*w, *h);
                }
                _ => {}
            }

            // Forward input to slave.
            if config.input.capture_mouse || config.input.capture_keyboard {
                if let Some(action) = translate_event(
                    ev,
                    win_width,
                    win_height,
                    remote_width,
                    remote_height,
                ) {
                    let result = match action {
                        InputAction::Mouse(me) => conn.send_mouse(&me).await,
                        InputAction::Key(ke) => conn.send_keyboard(&ke).await,
                    };
                    if let Err(e) = result {
                        warn!("failed to send input: {e}");
                    }
                }
            }
        }

        // Check for new frames.
        if frame_rx.has_changed().unwrap_or(false) {
            let frame_buf = frame_rx.borrow_and_update().clone();
            let stats = stats_rx.borrow().clone();

            if stats.width > 0 && stats.height > 0 {
                remote_width = stats.width;
                remote_height = stats.height;
            }

            if let Err(e) = renderer.render(&frame_buf, remote_width, remote_height) {
                warn!("render error: {e}");
            }
        }

        // Yield briefly so Tokio can make progress.
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }

    // ── 5. Shutdown ─────────────────────────────────────────────

    info!("shutting down");
    client_handle.abort();
    let _ = client_handle.await;
    drop(conn);

    Ok(())
}
