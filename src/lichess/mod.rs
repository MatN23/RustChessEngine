//! Lichess Bot API integration, written in Rust.

pub mod bot;
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