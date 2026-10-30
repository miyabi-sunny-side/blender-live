mod cli;
mod logging;
mod port;

use blender_live::{
    AppState,
    basecamp::BaseCamp,
    blender::{Blender, Config},
    events,
};
use std::{error::Error, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let camp = match cli::parse(&args) {
        Ok(cli::Cli::Serve(camp)) => camp,
        Ok(cli::Cli::Version) => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(cli::Cli::Help) => {
            println!("{}", cli::USAGE);
            return Ok(());
        }
        Err(e) => {
            eprintln!("{e}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    logging::init();

    let camp = BaseCamp::open(&camp)?;
    let bin =
        std::env::var_os("BLENDER_BIN").map_or_else(|| PathBuf::from("blender"), PathBuf::from);
    let gpus = cli::gpus(std::env::var("BLENDER_GPUS").ok().as_deref())?;
    let port = port::from_env()?;
    info!(camp = %camp.root().display(), bin = %bin.display(), ?gpus, "starting Blender");

    let bus = events::bus();
    let _watcher = events::watch_files(camp.root(), bus.clone())?;
    let blender = Blender::start(Config {
        bin,
        gpus,
        camp,
        grace: Duration::from_secs(30),
    })
    .await?;
    events::forward_status(blender.subscribe(), bus.clone());
    let state = AppState::new(blender, bus);

    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");

    axum::serve(listener, blender_live::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
