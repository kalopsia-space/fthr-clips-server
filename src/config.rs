use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeMode {
    Off,
    Compat,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderChoice {
    Auto,
    X264,
    Nvenc,
    Vaapi,
    Qsv,
    VideoToolbox,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub video_dirs: Vec<PathBuf>,
    pub image_dirs: Vec<PathBuf>,
    pub upload_token: Option<String>,
    pub admin_password: Option<String>,
    pub admin_password_hash: Option<String>,
    pub public_base_url: Option<String>,
    pub max_upload_bytes: u64,
    pub trusted_proxies: Vec<ipnet::IpNet>,
    pub scan_interval_minutes: u64,
    pub transcoding: TranscodeMode,
    pub encoder: EncoderChoice,
    pub transcode_workers: usize,
    pub transcode_timeout_minutes: u64,
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    pub discord_webhook_url: Option<String>,
    pub webhook_url: Option<String>,
    pub site_title: String,
    pub log_level: String,
}

impl Config {
    /// Load configuration: defaults < `{data_dir}/config.toml` < environment.
    pub fn load() -> anyhow::Result<Config> {
        let env: HashMap<String, String> = std::env::vars().collect();
        let data_dir = Self::data_dir_from(&env);
        let toml_path = data_dir.join("config.toml");
        let toml_text = if toml_path.exists() {
            Some(
                std::fs::read_to_string(&toml_path)
                    .with_context(|| format!("reading {}", toml_path.display()))?,
            )
        } else {
            None
        };
        if toml_text.is_none() && env.get("UPLOAD_TOKEN").is_none_or(|v| v.is_empty()) {
            bail!(
                "UPLOAD_TOKEN is not set and {} does not exist; set UPLOAD_TOKEN or create a config.toml",
                toml_path.display()
            );
        }
        Self::from_env_map(&env, toml_text.as_deref())
    }

    /// Resolve the data dir: env DATA_DIR, env STORAGE_DIR, toml data_dir, toml storage_dir.
    fn resolve_data_dir(
        env: &HashMap<String, String>,
        toml: &HashMap<String, String>,
    ) -> Option<PathBuf> {
        [
            (env, "DATA_DIR"),
            (env, "STORAGE_DIR"),
            (toml, "DATA_DIR"),
            (toml, "STORAGE_DIR"),
        ]
        .into_iter()
        .filter_map(|(m, k)| m.get(k))
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
    }

    fn data_dir_from(env: &HashMap<String, String>) -> PathBuf {
        Self::resolve_data_dir(env, &HashMap::new()).unwrap_or_else(|| PathBuf::from("data"))
    }

    /// Build a config from an explicit environment map and optional TOML text.
    /// Environment values override TOML values; TOML keys are the lowercase env names.
    pub fn from_env_map(
        vars: &HashMap<String, String>,
        toml: Option<&str>,
    ) -> anyhow::Result<Config> {
        let mut m: HashMap<String, String> = HashMap::new();
        if let Some(text) = toml {
            let table: toml::Table = text.parse().context("parsing config.toml")?;
            for (k, v) in table {
                let s = match v {
                    toml::Value::String(s) => s,
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Boolean(b) => b.to_string(),
                    toml::Value::Float(f) => f.to_string(),
                    toml::Value::Array(items) => items
                        .iter()
                        .map(|i| match i {
                            toml::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(","),
                    other => bail!("unsupported value for `{k}` in config.toml: {other}"),
                };
                m.insert(k.to_uppercase(), s);
            }
        }
        let data_dir = Self::resolve_data_dir(vars, &m);
        m.extend(vars.iter().map(|(k, v)| (k.clone(), v.clone())));

        let get = |k: &str| m.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
        let text = |k: &str| get(k).map(str::to_owned);
        let list = |k: &str| -> Vec<PathBuf> {
            get(k)
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default()
        };
        fn num<T: std::str::FromStr>(k: &str, v: Option<&str>, default: T) -> anyhow::Result<T> {
            match v {
                None => Ok(default),
                Some(s) => s
                    .parse()
                    .map_err(|_| anyhow!("invalid value for {k}: {s:?}")),
            }
        }

        let mut trusted_proxies = Vec::new();
        if let Some(s) = get("TRUSTED_PROXIES") {
            for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                let net = part
                    .parse::<ipnet::IpNet>()
                    .or_else(|_| part.parse::<std::net::IpAddr>().map(ipnet::IpNet::from))
                    .map_err(|_| anyhow!("invalid TRUSTED_PROXIES entry: {part:?}"))?;
                trusted_proxies.push(net);
            }
        }

        let transcoding = match get("TRANSCODING").map(str::to_lowercase).as_deref() {
            None | Some("compat") => TranscodeMode::Compat,
            Some("off") => TranscodeMode::Off,
            Some("on") => TranscodeMode::On,
            Some(o) => bail!("invalid TRANSCODING {o:?} (expected off, compat or on)"),
        };
        let encoder = match get("ENCODER").map(str::to_lowercase).as_deref() {
            None | Some("auto") => EncoderChoice::Auto,
            Some("x264") => EncoderChoice::X264,
            Some("nvenc") => EncoderChoice::Nvenc,
            Some("vaapi") => EncoderChoice::Vaapi,
            Some("qsv") => EncoderChoice::Qsv,
            Some("videotoolbox") => EncoderChoice::VideoToolbox,
            Some(o) => bail!("invalid ENCODER {o:?}"),
        };

        Ok(Config {
            bind_host: text("BIND_HOST").unwrap_or_else(|| "127.0.0.1".into()),
            port: num("PORT", get("PORT"), 8080)?,
            data_dir: data_dir.unwrap_or_else(|| PathBuf::from("data")),
            video_dirs: list("VIDEO_DIRS"),
            image_dirs: list("IMAGE_DIRS"),
            upload_token: text("UPLOAD_TOKEN"),
            admin_password: text("ADMIN_PASSWORD"),
            admin_password_hash: text("ADMIN_PASSWORD_HASH"),
            public_base_url: text("PUBLIC_BASE_URL"),
            max_upload_bytes: num("MAX_UPLOAD_BYTES", get("MAX_UPLOAD_BYTES"), 524_288_000)?,
            trusted_proxies,
            scan_interval_minutes: num("SCAN_INTERVAL_MINUTES", get("SCAN_INTERVAL_MINUTES"), 15)?,
            transcoding,
            encoder,
            transcode_workers: num("TRANSCODE_WORKERS", get("TRANSCODE_WORKERS"), 1)?,
            transcode_timeout_minutes: num(
                "TRANSCODE_TIMEOUT_MINUTES",
                get("TRANSCODE_TIMEOUT_MINUTES"),
                60,
            )?,
            ffmpeg_path: text("FFMPEG_PATH").map(PathBuf::from),
            ffprobe_path: text("FFPROBE_PATH").map(PathBuf::from),
            discord_webhook_url: text("DISCORD_WEBHOOK_URL"),
            webhook_url: text("WEBHOOK_URL"),
            site_title: text("SITE_TITLE").unwrap_or_else(|| "FTHR Clips".into()),
            log_level: text("LOG_LEVEL").unwrap_or_else(|| "info".into()),
        })
    }
}
