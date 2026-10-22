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
    /// Print environment diagnostics (data dir, database, ffmpeg, encoder, scan dirs)
    Check,
    /// Scan the configured folders once, print the report and exit
    Scan,
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
        Cmd::Check => check(config).await,
        Cmd::Scan => scan(config).await,
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

async fn check(config: Config) -> anyhow::Result<()> {
    use fthr_clips_server::{
        db::Db,
        ffmpeg::{encoders, Ffmpeg},
    };
    println!("data dir: {}", config.data_dir.display());
    std::fs::create_dir_all(&config.data_dir)?;
    Db::open(&config.data_dir.join("library.db")).await?;
    println!("db: ok");
    let ff = Ffmpeg::locate(&config);
    match &ff.ffmpeg {
        Some(p) => println!(
            "ffmpeg: {} ({})",
            p.display(),
            ff.version.as_deref().unwrap_or("unknown version")
        ),
        None => println!("ffmpeg: not found"),
    }
    match &ff.ffprobe {
        Some(p) => println!("ffprobe: {}", p.display()),
        None => println!("ffprobe: not found"),
    }
    match encoders::detect(&ff, config.encoder).await {
        Some(e) => println!("encoder: {}", e.name()),
        None => println!("encoder: none (transcoding unavailable)"),
    }
    for d in config.video_dirs.iter().chain(config.image_dirs.iter()) {
        let state = if d.is_dir() { "exists" } else { "missing" };
        println!("scan dir: {} ({state})", d.display());
    }
    println!("bind: {}:{}", config.bind_host, config.port);
    Ok(())
}

async fn scan(config: Config) -> anyhow::Result<()> {
    use fthr_clips_server::{app::AppState, db::Db, media::scan::Scanner, settings::SettingsStore};
    use std::sync::Arc;
    std::fs::create_dir_all(&config.data_dir)?;
    let db = Db::open(&config.data_dir.join("library.db")).await?;
    let settings = Arc::new(SettingsStore::load(&db, &config).await?);
    let state = AppState::new(config, db, settings);
    let report = Scanner::new(state).run_once().await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
