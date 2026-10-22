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
    /// Read a password from stdin and print its argon2id hash
    HashPassword,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(Cmd::HashPassword) = cli.cmd {
        return hash_password();
    }
    let config = Config::load()?;
    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => app::serve(config).await,
        Cmd::HashPassword => unreachable!("handled above"),
    }
}

fn hash_password() -> anyhow::Result<()> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let password = line.trim_end_matches(['\r', '\n']);
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hashing failed: {e}"))?;
    println!("{hash}");
    Ok(())
}
