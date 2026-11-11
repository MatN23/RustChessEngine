//! Event loop: challenges in, games out.

use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::challenger::{Action, ChallengeSettings, Challenger};
use super::client::{ApiError, LichessClient, StreamItem};
use super::game::{play_game, GameContext, GameOutcome, THREAD_STACK};
use super::Config;
use crate::search::SearchEngine;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Decline(&'static str),
}

/// Pure challenge policy (unit-tested).
///
/// * Only standard chess (`fromPosition` is standard rules from a custom FEN).
/// * No unlimited / correspondence games: the clock-based time formula assumes
///   a real clock, and such a game would pin the single game slot for days.
/// * Respect the concurrent-game limit.
pub fn decide_challenge(ch: &Value, slots_free: bool) -> Decision {
    let variant = ch["variant"]["key"].as_str().unwrap_or("standard");
    if variant != "standard" && variant != "fromPosition" {
        return Decision::Decline("variant");
    }
    let tc_type = ch["timeControl"]["type"].as_str().unwrap_or("clock");
    let speed = ch["speed"].as_str().unwrap_or("");
    if tc_type == "unlimited" || tc_type == "correspondence" || speed == "correspondence" {
        return Decision::Decline("timeControl");
    }
    if !slots_free {
        return Decision::Decline("later");
    }
    Decision::Accept
}

/// Game slot bookkeeping. On Lichess the game id equals the challenge id, so
/// accepting a challenge *reserves* a slot until its `gameStart` arrives;
/// otherwise two challenges arriving back-to-back would both be accepted.
#[derive(Default)]
pub struct Games {
    reserved: HashSet<String>,
    playing: HashSet<String>,
}

impl Games {
    pub fn count(&self) -> usize {
        self.reserved.len() + self.playing.len()
    }
    pub fn try_reserve(&mut self, id: &str, max: usize) -> bool {
        if self.count() >= max && !self.reserved.contains(id) && !self.playing.contains(id) {
            return false;
        }
        self.reserved.insert(id.to_string());
        true
    }
    pub fn release_reservation(&mut self, id: &str) {
        self.reserved.remove(id);
    }
    /// Returns false if this game is already being played.
    pub fn start_playing(&mut self, id: &str) -> bool {
        if self.playing.contains(id) {
            return false;
        }
        self.reserved.remove(id);
        self.playing.insert(id.to_string());
        true
    }
    pub fn finish(&mut self, id: &str) {
        self.playing.remove(id);
        self.reserved.remove(id);
    }
}

#[derive(Default)]
pub struct BotStats {
    pub finished: AtomicUsize,
    pub abandoned: AtomicUsize,
    pub panics: AtomicUsize,
    pub accepted: AtomicUsize,
    pub declined: AtomicUsize,
}

pub struct Bot {
    ctx: GameContext,
    games: Arc<Mutex<Games>>,
    stats: Arc<BotStats>,
    challenger: Arc<Mutex<Challenger>>,
}

impl Bot {
    /// Validates the token (fails fast) and builds the engine.
    pub fn connect(cfg: Config) -> Result<Bot, String> {
        cfg.validate()?;
        let client = Arc::new(LichessClient::new(&cfg.base_url, &cfg.token));
        let account = client.account().map_err(|e| {
            format!("could not read /api/account ({e}); is the token valid and a BOT account?")
        })?;
        println!("Logged in as {} ({}/@/{})", account.username, cfg.base_url, account.username);

        println!(
            "Initializing engine: {} thread(s), {} MB hash, ponder {}, full time budget {}",
            cfg.threads,
            cfg.hash_mb,
            if cfg.ponder { "on" } else { "off" },
            if cfg.full_budget { "on" } else { "off" }
        );
        let engine = Arc::new(SearchEngine::new(cfg.threads));
        engine.set_hash_size(cfg.hash_mb);
        engine.set_full_budget(cfg.full_budget);

        let challenger = Arc::new(Mutex::new(Challenger::new(
            ChallengeSettings {
                targets: cfg.challenge_bots.clone(),
                limit_secs: cfg.challenge_clock.0,
                increment_secs: cfg.challenge_clock.1,
                rated: cfg.challenge_rated,
                interval: Duration::from_secs(cfg.challenge_interval_secs),
                max_per_hour: cfg.challenge_max_per_hour,
            },
            &account.id,
        )));

        Ok(Bot {
            ctx: GameContext {
                client,
                engine,
                cfg: Arc::new(cfg),
                my_id: account.id,
            },
            games: Arc::new(Mutex::new(Games::default())),
            stats: Arc::new(BotStats::default()),
            challenger,
        })
    }

    pub fn challenger(&self) -> Arc<Mutex<Challenger>> {
        Arc::clone(&self.challenger)
    }

    pub fn stats(&self) -> Arc<BotStats> {
        Arc::clone(&self.stats)
    }

    pub fn games(&self) -> Arc<Mutex<Games>> {
        Arc::clone(&self.games)
    }

    /// Runs until `shutdown` is set. Returns `Err` only for unrecoverable
    /// problems (revoked / invalid token).
    pub fn run(&self, shutdown: &AtomicBool) -> Result<(), String> {
        if !self.ctx.cfg.auto_challenge {
            return self.run_events(shutdown);
        }
        // Opt-in: a second thread that challenges other bots while the event loop runs.
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            scope.spawn(|| self.challenge_loop(shutdown, &stop));
            let result = self.run_events(shutdown);
            stop.store(true, Ordering::Relaxed);
            result
        })
    }

    fn run_events(&self, shutdown: &AtomicBool) -> Result<(), String> {
        println!("Bot running, waiting for challenges (Ctrl+C to stop)");
        let mut backoff = 1u64;

        while !shutdown.load(Ordering::Relaxed) {
            match self.ctx.client.open_event_stream() {
                Ok(mut stream) => {
                    backoff = 1;
                    loop {
                        if shutdown.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        match stream.next_item() {
                            None => {
                                eprintln!("event stream closed by server");
                                break;
                            }
                            Some(Err(e)) => {
                                eprintln!("event stream error: {e}");
                                break;
                            }
                            Some(Ok(StreamItem::KeepAlive)) => {}
                            Some(Ok(StreamItem::Event(ev))) => self.process_event(&ev),
                        }
                    }
                }
                Err(ApiError::Status(code @ (401 | 403), body)) => {
                    return Err(format!("HTTP {code} on event stream: {body}"));
                }
                Err(ApiError::Status(429, _)) => {
                    eprintln!("rate limited (429) on event stream; waiting 60s");
                    backoff = 60;
                }
                Err(e) => eprintln!("cannot open event stream: {e}"),
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            eprintln!("reconnecting in {backoff}s...");
            sleep_unless(shutdown, Duration::from_secs(backoff));
            backoff = (backoff * 2).min(30);
        }
        Ok(())
    }

    pub fn process_event(&self, ev: &Value) {
        match ev["type"].as_str() {
            Some("challenge") => self.on_challenge(&ev["challenge"]),
            Some("challengeCanceled") | Some("challengeDeclined") => {
                if let Some(id) = ev["challenge"]["id"].as_str() {
                    self.games.lock().unwrap().release_reservation(id);
                    let mut ch = self.challenger.lock().unwrap();
                    if ev["type"].as_str() == Some("challengeDeclined") {
                        let reason = ev["challenge"]["declineReasonKey"].as_str().unwrap_or("generic");
                        if let Some(name) = ch.on_declined(id, reason, Instant::now()) {
                            println!("{name} declined our challenge ({reason}); leaving them alone for a while");
                        }
                    } else {
                        ch.on_resolved(id);
                    }
                }
            }
            Some("gameStart") => {
                if let Some(id) = ev["game"]["gameId"].as_str().or_else(|| ev["game"]["id"].as_str()) {
                    self.challenger.lock().unwrap().on_resolved(id);
                }
                self.on_game_start(&ev["game"])
            }
            Some("gameFinish") => {
                println!("game {} finished", ev["game"]["id"].as_str().unwrap_or("?"));
            }
            _ => {}
        }
    }

    fn on_challenge(&self, ch: &Value) {
        let Some(id) = ch["id"].as_str() else { return };
        let challenger = ch["challenger"]["name"].as_str().unwrap_or("?");
        if ch["challenger"]["id"].as_str().map(str::to_lowercase).as_deref()
            == Some(self.ctx.my_id.as_str())
        {
            return; // our own outgoing challenge
        }

        let slots_free = self.games.lock().unwrap().count() < self.ctx.cfg.max_games;
        match decide_challenge(ch, slots_free) {
            Decision::Accept => {
                // Re-check atomically while reserving.
                if !self
                    .games
                    .lock()
                    .unwrap()
                    .try_reserve(id, self.ctx.cfg.max_games)
                {
                    self.decline(id, "later", challenger);
                    return;
                }
                match self.ctx.client.accept_challenge(id) {
                    Ok(()) => {
                        self.stats.accepted.fetch_add(1, Ordering::Relaxed);
                        println!("accepted challenge {id} from {challenger}");
                    }
                    Err(e) => {
                        self.games.lock().unwrap().release_reservation(id);
                        eprintln!("could not accept challenge {id}: {e}");
                    }
                }
            }
            Decision::Decline(reason) => self.decline(id, reason, challenger),
        }
    }

    fn decline(&self, id: &str, reason: &str, challenger: &str) {
        self.stats.declined.fetch_add(1, Ordering::Relaxed);
        println!("declining challenge {id} from {challenger} ({reason})");
        if let Err(e) = self.ctx.client.decline_challenge(id, reason) {
            eprintln!("could not decline challenge {id}: {e}");
        }
    }

    fn on_game_start(&self, game: &Value) {
        let Some(id) = game["gameId"].as_str().or_else(|| game["id"].as_str()) else {
            return;
        };
        if !self.games.lock().unwrap().start_playing(id) {
            return; // already playing (event stream replays ongoing games on reconnect)
        }

        println!("game started: {}/{id}", self.ctx.cfg.base_url);
        let ctx = self.ctx.clone();
        let games = Arc::clone(&self.games);
        let stats = Arc::clone(&self.stats);
        let id_owned = id.to_string();

        let spawned = thread::Builder::new()
            .name(format!("game-{id}"))
            .stack_size(THREAD_STACK)
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| play_game(&ctx, &id_owned)));
                match outcome {
                    Ok(GameOutcome::Finished) => {
                        stats.finished.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(GameOutcome::Abandoned(why)) => {
                        eprintln!("[{id_owned}] abandoned: {why}");
                        stats.abandoned.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => {
                        eprintln!("[{id_owned}] game thread PANICKED");
                        stats.panics.fetch_add(1, Ordering::SeqCst);
                    }
                }
                games.lock().unwrap().finish(&id_owned);
            });
        if let Err(e) = spawned {
            eprintln!("could not spawn game thread: {e}");
            self.games.lock().unwrap().finish(id);
        }
    }
}

impl Bot {
    /// Background loop for AUTO_CHALLENGE: asks the pure `Challenger` what to do and performs it.
    fn challenge_loop(&self, shutdown: &AtomicBool, stop: &AtomicBool) {
        let cfg = &self.ctx.cfg;
        let (limit, inc) = cfg.challenge_clock;
        println!(
            "auto-challenge on: {}+{} {}, every {} s at most, {} per hour, opponents: {}",
            limit,
            inc,
            if cfg.challenge_rated { "rated" } else { "casual" },
            cfg.challenge_interval_secs,
            cfg.challenge_max_per_hour,
            if cfg.challenge_bots.is_empty() { "any online bot".to_string() } else { cfg.challenge_bots.join(", ") }
        );
        let mut cached: Vec<String> = Vec::new();
        let mut cached_at: Option<Instant> = None;
        let done = || shutdown.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);

        while !done() {
            let step = Duration::from_millis(500);
            thread::sleep(step);
            let now = Instant::now();
            let slots_free = self.games.lock().unwrap().count() < cfg.max_games;

            let candidates: Vec<String> = if !cfg.challenge_bots.is_empty() {
                cfg.challenge_bots.clone()
            } else {
                // Refresh every 3 minutes, or sooner (after 1 minute) when everybody we know about
                // is blocked, so newly online bots are found instead of cycling through the same ones.
                let age = cached_at.map_or(Duration::from_secs(u64::MAX / 4), |t| now.duration_since(t));
                let exhausted = !cached.is_empty()
                    && self.challenger.lock().unwrap().available(&cached, now) == 0;
                let stale = age > Duration::from_secs(180) || (exhausted && age > Duration::from_secs(60));
                if stale && slots_free {
                    match self.ctx.client.online_bots(200) {
                        Ok(v) => {
                            println!("{} bots online", v.len());
                            cached = v;
                            cached_at = Some(now);
                        }
                        Err(e) => {
                            eprintln!("could not list online bots: {e}");
                            cached.clear();
                            // retry in about a minute
                            cached_at = Some(now - Duration::from_secs(540));
                        }
                    }
                }
                cached.clone()
            };

            let action = self.challenger.lock().unwrap().next_action(now, slots_free, &candidates);
            match action {
                Action::Wait => {}
                Action::Cancel(id) => {
                    if let Err(e) = self.ctx.client.cancel_challenge(&id) {
                        eprintln!("could not cancel challenge {id}: {e}");
                    }
                    self.games.lock().unwrap().release_reservation(&id);
                    self.challenger.lock().unwrap().on_timed_out(&id, now);
                    println!("challenge {id} was not answered in time; cancelled");
                }
                Action::Send(name) => match self.ctx.client.create_challenge(&name, cfg.challenge_rated, limit, inc) {
                    Ok(id) => {
                        // The game id equals the challenge id: hold a game slot until it resolves.
                        if self.games.lock().unwrap().try_reserve(&id, cfg.max_games) {
                            self.challenger.lock().unwrap().on_sent(&id, &name, now);
                            println!("challenged {name} ({limit}+{inc}, challenge {id})");
                        } else {
                            let _ = self.ctx.client.cancel_challenge(&id);
                            println!("no free game slot for challenge {id}; cancelled");
                        }
                    }
                    Err(e) => {
                        let status = if let ApiError::Status(code, _) = &e { Some(*code) } else { None };
                        let retry_after = e.rate_limit_seconds();
                        self.challenger.lock().unwrap().on_send_failed(&name, status, retry_after, now);
                        match retry_after {
                            Some(secs) => println!("{name} is rate limited by Lichess for {} min; skipping them", secs / 60),
                            None => eprintln!("could not challenge {name}: {e}"),
                        }
                    }
                },
            }
        }
    }
}

fn sleep_unless(shutdown: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < total && !shutdown.load(Ordering::Relaxed) {
        thread::sleep(step);
        waited += step;
    }
}
