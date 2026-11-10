//! Lichess Bot API integration, written in Rust.

pub mod bot;
pub mod challenger;
pub mod client;
pub mod game;
pub mod timing;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timing_cases;

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Clone, Debug)]
pub struct Config {
    pub token: String,
    pub base_url: String,
    pub threads: usize,
    pub hash_mb: usize,
    /// Network/latency safety margin subtracted from every move's time budget.
    pub move_overhead_ms: u64,
    /// Think time per move in games that have no clock at all (unlimited time control).
    /// `None` (the default) keeps the legacy behaviour of assuming a 60 s clock (~2 s/move).
    pub unlimited_move_ms: Option<u64>,
    /// Use the whole per-move budget (ENGINE_FULL_BUDGET=1). Off by default.
    pub full_budget: bool,
    pub search_depth: u8,
    pub ponder: bool,
    /// Upper bound for a single pondering search.
    pub ponder_max_ms: u64,
    /// Games played at once. The engine has one search lock and one TT, so
    /// values above 1 make games steal each other's clock time.
    pub max_games: usize,
    /// Opt-in: challenge other bots automatically (AUTO_CHALLENGE=1).
    pub auto_challenge: bool,
    /// Fixed opponents (CHALLENGE_BOTS=a,b,c). Empty = pick from the bots that are online.
    pub challenge_bots: Vec<String>,
    /// (seconds, increment) of the games we offer (CHALLENGE_TIME=180+2).
    pub challenge_clock: (u32, u32),
    pub challenge_rated: bool,
    /// Minimum seconds between two challenges we send.
    pub challenge_interval_secs: u64,
    pub challenge_max_per_hour: u32,
}

impl Config {
    /// Same variables as the old Python bot (`LICHESS_TOKEN`, `ENGINE_THREADS`,
    /// `ENGINE_HASH`) with the same defaults, plus a few new knobs.
    pub fn from_env(args: &[String]) -> Result<Config, String> {
        let token = std::env::var("LICHESS_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| args.first().cloned())
            .ok_or_else(|| "no Lichess token given".to_string())?;

        let cfg = Config {
            token: token.trim().to_string(),
            base_url: std::env::var("LICHESS_URL")
                .unwrap_or_else(|_| "https://lichess.org".to_string())
                .trim_end_matches('/')
                .to_string(),
            threads: env_parse("ENGINE_THREADS", 8)?,
            hash_mb: env_parse("ENGINE_HASH", 2048)?,
            move_overhead_ms: env_parse("MOVE_OVERHEAD", 300)?,
            unlimited_move_ms: match std::env::var("UNLIMITED_MOVE_TIME_MS") {
                Err(_) => None,
                Ok(v) => Some(
                    v.trim()
                        .parse::<u64>()
                        .map_err(|_| format!("UNLIMITED_MOVE_TIME_MS={v:?} is not a valid value"))?,
                ),
            },
            full_budget: matches!(
                std::env::var("ENGINE_FULL_BUDGET").as_deref().map(str::trim),
                Ok("1") | Ok("true") | Ok("TRUE") | Ok("on") | Ok("ON")
            ),
            search_depth: env_parse("SEARCH_DEPTH", 32)?,
            ponder: env_parse::<u8>("PONDER", 1)? != 0,
            ponder_max_ms: env_parse("PONDER_MAX_MS", 20_000)?,
            max_games: env_parse("MAX_GAMES", 1)?,
            auto_challenge: matches!(
                std::env::var("AUTO_CHALLENGE").as_deref().map(str::trim),
                Ok("1") | Ok("true") | Ok("TRUE") | Ok("on") | Ok("ON")
            ),
            challenge_bots: std::env::var("CHALLENGE_BOTS")
                .unwrap_or_default()
                .split(',')
                .map(|n| n.trim().to_lowercase())
                .filter(|n| !n.is_empty())
                .collect(),
            challenge_clock: challenger::parse_clock(
                &std::env::var("CHALLENGE_TIME").unwrap_or_else(|_| "180+2".to_string()),
            )?,
            challenge_rated: matches!(
                std::env::var("CHALLENGE_RATED").as_deref().map(str::trim),
                Ok("1") | Ok("true") | Ok("TRUE") | Ok("on") | Ok("ON")
            ),
            challenge_interval_secs: env_parse("CHALLENGE_INTERVAL", 30)?,
            challenge_max_per_hour: env_parse("CHALLENGE_MAX_PER_HOUR", 10)?,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.threads == 0 {
            return Err("ENGINE_THREADS must be >= 1".into());
        }
        if self.hash_mb == 0 {
            return Err("ENGINE_HASH must be >= 1 (MB)".into());
        }
        if self.max_games == 0 {
            return Err("MAX_GAMES must be >= 1".into());
        }
        if self.search_depth == 0 {
            return Err("SEARCH_DEPTH must be >= 1".into());
        }
        if self.auto_challenge {
            if self.challenge_interval_secs < 5 {
                return Err("CHALLENGE_INTERVAL must be at least 5 seconds".into());
            }
            if self.challenge_max_per_hour == 0 {
                return Err("CHALLENGE_MAX_PER_HOUR must be >= 1".into());
            }
            if let Some(bad) = self.challenge_bots.iter().find(|n| !client::valid_username(n)) {
                return Err(format!("CHALLENGE_BOTS: {bad:?} is not a valid Lichess username"));
            }
        }
        if !(self.base_url.starts_with("https://") || self.base_url.starts_with("http://")) {
            return Err("LICHESS_URL must start with http:// or https://".into());
        }
        Ok(())
    }
}

/// Parse an env var, falling back to `default` when unset. A value that is set
/// but malformed is an error (silently ignoring a typo'd setting is worse).
fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => v
            .trim()
            .parse::<T>()
            .map_err(|_| format!("{name}={v:?} is not a valid value")),
    }
}
