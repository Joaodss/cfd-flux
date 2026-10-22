//! `cfd-server`: the cfd-flux API on `127.0.0.1:7878` (local-first, no authentication; ADR-010).

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use cfd_engines::{EngineOptions, Inventory};
use cfd_server::web::{self, Web};
use cfd_server::{AppState, Config};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "cfd-server", version, about = "cfd-flux simulation server")]
struct Args {
    /// Address and port to listen on. Anything but a loopback address exposes the server, which
    /// has no authentication, to the network.
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: SocketAddr,
    /// Directory for the database and the results (`results/<job id>/`).
    #[arg(long, default_value = "data")]
    data: PathBuf,
    /// Database URL (default: SQLite in the data directory).
    #[arg(long)]
    database: Option<String>,
    /// CPU jobs that may run at the same time (they share one thread pool).
    #[arg(long, default_value_t = 1)]
    cpu_jobs: usize,
    /// Number of CPU threads (default: all logical cores).
    #[arg(long)]
    threads: Option<usize>,
    /// Let jobs that ask for wgpu use a software adapter when there is no GPU.
    #[arg(long)]
    allow_software: bool,
    /// Serve the web app from this directory (the output of `npm run build`) instead of the one
    /// compiled into the binary.
    #[arg(long, value_name = "DIR")]
    web_dir: Option<PathBuf>,
    /// Open the web app in the default browser once the server is listening.
    #[arg(long)]
    open: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs: `RUST_LOG=cfd_server=debug,tower_http=debug` for more detail.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("cfd_server=info,tower_http=info")),
        )
        .init();
    let args = Args::parse();
    cfd_engines::configure_threads(args.threads)?;
    let engine = EngineOptions {
        allow_software: args.allow_software,
        ..Default::default()
    };
    // Opening devices blocks (driver calls, adapter enumeration): run it off the async threads.
    let inventory = tokio::task::spawn_blocking(move || Inventory::probe(&engine)).await?;
    for e in inventory.engines() {
        match (&e.device, &e.reason) {
            (Some(d), _) => tracing::info!("engine {}: {d}", e.kind),
            (None, Some(r)) if e.available => tracing::info!("engine {}: {r}", e.kind),
            (None, Some(r)) => tracing::info!("engine {} unavailable: {r}", e.kind),
            (None, None) => tracing::info!("engine {}", e.kind),
        }
    }
    let web = match (args.web_dir, Web::EMBEDDED_AVAILABLE) {
        (Some(dir), _) => Web::Dir(dir),
        (None, true) => Web::Embedded,
        (None, false) => Web::None,
    };
    match &web {
        Web::None => tracing::info!(
            "no web app: the API only (build with --features web, or give --web-dir)"
        ),
        Web::Dir(d) => tracing::info!("web app from {}", d.display()),
        Web::Embedded => tracing::info!("web app embedded in the binary"),
    }
    let open = args.open && !matches!(web, Web::None);
    if args.open && !open {
        tracing::warn!("--open: there is no web app to open");
    }
    let config = Config {
        listen: args.listen,
        data_dir: args.data,
        database_url: args.database,
        cpu_jobs: args.cpu_jobs.max(1),
        web,
        engine,
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    let url = web::browser_url(listener.local_addr()?);
    let state = AppState::new(config, inventory).await?;
    if open {
        // The socket is bound, so the browser can connect at once (it waits in the accept queue
        // for the few milliseconds until `serve` runs).
        web::open_browser(url);
    }
    cfd_server::serve(listener, state, async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    })
    .await
}
