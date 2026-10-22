use clap::{Parser, Subcommand};
use fthr_clips_server::{app, config::Config};

#[derive(Parser)]
#[command(version, about = "FTHR clip sharing server")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the web server (default)
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => app::serve(config).await,
    }
}
