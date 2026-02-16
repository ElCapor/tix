//! TIX RDP Slave — entry point.
//!
//! ```text
//! tix-rdp-slave                  Run as console (foreground)
//! tix-rdp-slave --install        Install as Windows service
//! tix-rdp-slave --uninstall      Remove Windows service
//! tix-rdp-slave --config <path>  Load a custom config TOML
//! tix-rdp-slave --gen-config     Write default config to stdout
//! ```

use std::path::PathBuf;

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use tix_rdp_slave::config::SlaveConfig;
use tix_rdp_slave::service::RdpSlaveService;

// ── CLI ──────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "tix-rdp-slave", about = "TIX RDP slave screen-capture service")]
struct Cli {
    /// Path to configuration TOML file.
    #[arg(short, long, default_value = "tix-rdp-slave.toml")]
    config: PathBuf,

    /// Install as a Windows service.
    #[arg(long)]
    install: bool,

    /// Uninstall the Windows service.
    #[arg(long)]
    uninstall: bool,

    /// Print the default configuration to stdout and exit.
    #[arg(long)]
    gen_config: bool,
}

// ── Main ─────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // --gen-config: dump defaults and exit.
    if cli.gen_config {
        let text = toml::to_string_pretty(&SlaveConfig::default())?;
        println!("{text}");
        return Ok(());
    }

    // --install / --uninstall: Windows service management.
    #[cfg(target_os = "windows")]
    {
        if cli.install {
            tix_rdp_slave::win_service::install_service()?;
            println!("Service installed.");
            return Ok(());
        }
        if cli.uninstall {
            tix_rdp_slave::win_service::uninstall_service()?;
            println!("Service uninstalled.");
            return Ok(());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if cli.install || cli.uninstall {
            eprintln!("Windows service management is only available on Windows.");
            std::process::exit(1);
        }
    }

    // Load config.
    let config = SlaveConfig::load(&cli.config);

    // Init tracing.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    info!("tix-rdp-slave v{}", env!("CARGO_PKG_VERSION"));
    info!("control port: {}", config.network.control_port);
    info!("screen UDP port: {}", config.network.listen_port);
    info!("target FPS: {}", config.screen.fps);
    info!("monitor: {}", config.screen.monitor_index);

    // Run in console mode.
    let service = RdpSlaveService::new(config);
    let stop = service.stop_handle();

    // Ctrl-C handler.
    let stop_clone = stop.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Ctrl-C received — shutting down");
        stop_clone.store(false, std::sync::atomic::Ordering::SeqCst);
    });

    service.run().await?;

    Ok(())
}
