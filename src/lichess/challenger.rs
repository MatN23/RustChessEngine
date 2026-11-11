//! Opt-in automatic challenges to other bots (`AUTO_CHALLENGE=1`).
//!
//! This file is pure decision logic (no network, no threads) so it can be tested exhaustively:
//! *when* to challenge, *whom*, and how to back off. The network calls live in `bot.rs`.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// A challenge nobody answered after this long is cancelled.
pub const PENDING_TIMEOUT: Duration = Duration::from_secs(60);
/// Pause after a 429 (Lichess rate limit).
pub const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(300);
/// Opponents that refused for a reason that will not change (not a bot account, wrong time
/// control, rated/casual only, ...) are left alone for the rest of the session.
const BLOCK_LONG: Duration = Duration::from_secs(24 * 3600);
/// A plain "no" (`generic`) is probably not about timing: try again much later.
const BLOCK_MEDIUM: Duration = Duration::from_secs(3600);
/// Opponents that were merely busy / unresponsive are retried later.
const BLOCK_SHORT: Duration = Duration::from_secs(15 * 60);
/// After a failed attempt, wait only this long before trying the next opponent.
const FAILED_RETRY_GAP: Duration = Duration::from_secs(5);
/// This many failures in a row (no challenge accepted by Lichess in between) pauses everything:
/// the limit is probably ours, not the opponents'.
const FAILURE_STREAK: u32 = 10;
const FAILURE_STREAK_PAUSE: Duration = Duration::from_secs(600);
const HOUR: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug)]
pub struct ChallengeSettings {
    /// Fixed opponents (lower-case). Empty = pick from the list of online bots.
    pub targets: Vec<String>,
    pub limit_secs: u32,
    pub increment_secs: u32,
    pub rated: bool,
    /// Minimum time between two challenges we send.
    pub interval: Duration,
    pub max_per_hour: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Wait,
    /// Send a challenge to this user.
    Send(String),
    /// Cancel our outstanding challenge with this id (nobody answered in time).
    Cancel(String),
}

struct Pending {
    id: String,
    name: String,
    since: Instant,
}

pub struct Challenger {
    settings: ChallengeSettings,
    my_id: String,
    blocked: HashMap<String, Instant>,
    pending: Option<Pending>,
    sent: VecDeque<Instant>,
    last_attempt: Option<Instant>,
    paused_until: Option<Instant>,
    failure_streak: u32,
    rng: u64,
}

impl Challenger {
    pub fn new(settings: ChallengeSettings, my_id: &str) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self::new_seeded(settings, my_id, seed)
    }

    /// Same as `new` with a fixed random seed (tests).
    pub fn new_seeded(settings: ChallengeSettings, my_id: &str, seed: u64) -> Self {
        Challenger {
            settings,
            my_id: my_id.to_lowercase(),
            blocked: HashMap::new(),
            pending: None,
            sent: VecDeque::new(),
            last_attempt: None,
            paused_until: None,
            failure_streak: 0,
            rng: seed | 1,
        }
    }

    fn rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn settings(&self) -> &ChallengeSettings {
        &self.settings
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn is_blocked(&self, name: &str, now: Instant) -> bool {
        self.blocked
            .get(&name.to_lowercase())
            .map_or(false, |until| *until > now)
    }

    fn block(&mut self, name: &str, now: Instant, how_long: Duration) {
        self.blocked.insert(name.to_lowercase(), now + how_long);
    }

    /// Decide what to do right now. `slots_free` is whether the bot has room for another game;
    /// `candidates` are the usernames we may challenge (fixed list or online bots).
    pub fn next_action(&mut self, now: Instant, slots_free: bool, candidates: &[String]) -> Action {
        if self.paused_until.map_or(false, |t| t > now) {
            return Action::Wait;
        }
        if let Some(p) = &self.pending {
            return if now.duration_since(p.since) >= PENDING_TIMEOUT {
                Action::Cancel(p.id.clone())
            } else {
                Action::Wait
            };
        }
        if !slots_free {
            return Action::Wait;
        }
        if let Some(t) = self.last_attempt {
            if now.duration_since(t) < self.settings.interval {
                return Action::Wait;
            }
        }
        while self.sent.front().map_or(false, |t| now.duration_since(*t) >= HOUR) {
            self.sent.pop_front();
        }
        if self.sent.len() >= self.settings.max_per_hour as usize {
            return Action::Wait;
        }
        let eligible: Vec<&String> = candidates
            .iter()
            .filter(|n| !n.eq_ignore_ascii_case(&self.my_id) && !self.is_blocked(n, now))
            .collect();
        if eligible.is_empty() {
            return Action::Wait;
        }
        // Random, not round-robin: a fixed order keeps hitting the same (often busiest) bots.
        let pick = eligible[(self.rand() % eligible.len() as u64) as usize].clone();
        self.last_attempt = Some(now);
        Action::Send(pick)
    }

    /// How many of `candidates` we could challenge right now.
    pub fn available(&self, candidates: &[String], now: Instant) -> usize {
        candidates
            .iter()
            .filter(|n| !n.eq_ignore_ascii_case(&self.my_id) && !self.is_blocked(n, now))
            .count()
    }

    /// The challenge was created (Lichess returned its id).
    pub fn on_sent(&mut self, id: &str, name: &str, now: Instant) {
        self.pending = Some(Pending {
            id: id.to_string(),
            name: name.to_string(),
            since: now,
        });
        self.sent.push_back(now);
        self.failure_streak = 0;
    }

    /// Creating the challenge failed. `status` is the HTTP status if there was one and
    /// `retry_after_secs` the wait Lichess asked for (`ratelimit.seconds`), if any.
    pub fn on_send_failed(&mut self, name: &str, status: Option<u16>, retry_after_secs: Option<u64>, now: Instant) {
        // A failed attempt must not cost the whole interval: try the next opponent soon.
        let gap = FAILED_RETRY_GAP.min(self.settings.interval);
        self.last_attempt = now
            .checked_sub(self.settings.interval)
            .map(|t| t + gap)
            .or(Some(now));
        self.failure_streak += 1;
        let retry = retry_after_secs.map(Duration::from_secs);
        if status == Some(429) {
            let pause = retry.unwrap_or(RATE_LIMIT_PAUSE).clamp(Duration::from_secs(60), HOUR);
            self.paused_until = Some(now + pause);
        } else if let Some(wait) = retry {
            // e.g. "X played 100 games against other bots today, please wait ...": skip that
            // opponent for exactly as long as Lichess says (plus a margin).
            self.block(name, now, (wait + Duration::from_secs(30)).min(BLOCK_LONG));
        } else {
            self.block(name, now, BLOCK_SHORT);
        }
        if self.failure_streak >= FAILURE_STREAK {
            self.failure_streak = 0;
            self.paused_until = Some(now + FAILURE_STREAK_PAUSE);
        }
    }

    /// The opponent declined. Returns the opponent's name if this was our pending challenge.
    pub fn on_declined(&mut self, id: &str, reason_key: &str, now: Instant) -> Option<String> {
        if self.pending.as_ref().map(|p| p.id.as_str()) != Some(id) {
            return None;
        }
        let p = self.pending.take().unwrap();
        self.block(&p.name, now, block_duration(reason_key));
        Some(p.name)
    }

    /// The challenge disappeared (cancelled by someone) or turned into a game: nothing pending.
    pub fn on_resolved(&mut self, id: &str) {
        if self.pending.as_ref().map(|p| p.id.as_str()) == Some(id) {
            self.pending = None;
        }
    }

    /// We cancelled our own unanswered challenge.
    pub fn on_timed_out(&mut self, id: &str, now: Instant) {
        if self.pending.as_ref().map(|p| p.id.as_str()) == Some(id) {
            let p = self.pending.take().unwrap();
            self.block(&p.name, now, BLOCK_SHORT);
        }
    }
}

/// How long to leave an opponent alone after it declined. Lichess sends the reason key in
/// lower case in the event stream (`nobot`, `casual`, ...) but documents it in camelCase
/// (`noBot`), so compare case-insensitively.
pub fn block_duration(reason_key: &str) -> Duration {
    match reason_key.to_ascii_lowercase().as_str() {
        "nobot" | "onlybot" | "standard" | "variant" | "rated" | "casual" | "timecontrol"
        | "toofast" | "tooslow" => BLOCK_LONG,
        "generic" => BLOCK_MEDIUM,
        // "later" and anything unknown: try again after a while.
        _ => BLOCK_SHORT,
    }
}

/// "180+2" -> (180, 2) (seconds for the clock, seconds of increment).
pub fn parse_clock(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("CHALLENGE_TIME={s:?} must look like 180+2 (seconds + increment)");
    let (a, b) = s.trim().split_once('+').ok_or_else(bad)?;
    let limit: u32 = a.trim().parse().map_err(|_| bad())?;
    let inc: u32 = b.trim().parse().map_err(|_| bad())?;
    if limit > 10_800 || inc > 180 || (limit == 0 && inc == 0) {
        return Err(format!(
            "CHALLENGE_TIME={s:?}: clock must be at most 10800 s, increment at most 180 s, and not both 0"
        ));
    }
    Ok((limit, inc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ChallengeSettings {
        ChallengeSettings {
            targets: vec![],
            limit_secs: 180,
            increment_secs: 2,
            rated: false,
            interval: Duration::from_secs(30),
            max_per_hour: 3,
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn challenger(seed: u64) -> Challenger {
        Challenger::new_seeded(settings(), "MyBot", seed)
    }

    fn send_target(a: Action) -> String {
        match a {
            Action::Send(n) => n,
            other => panic!("expected a send, got {other:?}"),
        }
    }

    #[test]
    fn never_picks_self_or_a_blocked_opponent() {
        for seed in 0..50u64 {
            let mut c = challenger(seed);
            let t0 = Instant::now();
            let cands = names(&["mybot", "alpha", "beta", "gamma"]);
            c.on_sent("id1", "alpha", t0);
            assert!(c.on_declined("id1", "nobot", t0).is_some());
            let n = send_target(c.next_action(t0 + Duration::from_secs(31), true, &cands));
            assert!(n == "beta" || n == "gamma", "picked {n}");
        }
    }

    #[test]
    fn picks_are_spread_over_everybody_not_a_fixed_prefix() {
        // The old round-robin over a fixed list kept hitting the same bots. Over many picks every
        // eligible opponent must come up, including ones at the END of a long list.
        let cands: Vec<String> = (0..200).map(|i| format!("bot{i}")).collect();
        let mut c = challenger(7);
        let mut seen = std::collections::HashSet::new();
        let mut now = Instant::now();
        for _ in 0..2000 {
            now += Duration::from_secs(40);
            if let Action::Send(n) = c.next_action(now, true, &cands) {
                seen.insert(n);
                c.on_sent("x", "whoever", now);
                c.on_resolved("x");
                c.sent.clear(); // ignore the hourly quota for this statistical test
            }
        }
        assert!(seen.len() > 150, "only {} of 200 opponents were ever tried", seen.len());
        assert!(seen.contains("bot199") || seen.contains("bot198") || seen.contains("bot197"), "tail of the list never tried");
    }

    #[test]
    fn waits_for_slot_interval_and_pending() {
        let mut c = challenger(1);
        let t0 = Instant::now();
        let cands = names(&["alpha"]);
        assert_eq!(c.next_action(t0, false, &cands), Action::Wait, "no free game slot");
        assert_eq!(c.next_action(t0, true, &cands), Action::Send("alpha".into()));
        c.on_sent("id1", "alpha", t0);
        assert_eq!(c.next_action(t0 + Duration::from_secs(5), true, &cands), Action::Wait, "pending");
        c.on_resolved("id1"); // it became a game
        assert!(!c.has_pending());
        assert_eq!(c.next_action(t0 + Duration::from_secs(5), true, &cands), Action::Wait, "interval");
        assert_eq!(c.next_action(t0 + Duration::from_secs(31), true, &cands), Action::Send("alpha".into()));
    }

    #[test]
    fn unanswered_challenge_is_cancelled_after_the_timeout_and_opponent_rested() {
        let mut c = challenger(2);
        let t0 = Instant::now();
        let cands = names(&["alpha", "beta"]);
        let first = send_target(c.next_action(t0, true, &cands));
        c.on_sent("id1", &first, t0);
        assert_eq!(c.next_action(t0 + PENDING_TIMEOUT - Duration::from_secs(1), true, &cands), Action::Wait);
        let t1 = t0 + PENDING_TIMEOUT;
        assert_eq!(c.next_action(t1, true, &cands), Action::Cancel("id1".into()));
        c.on_timed_out("id1", t1);
        assert!(c.is_blocked(&first, t1));
        let second = send_target(c.next_action(t1 + Duration::from_secs(31), true, &cands));
        assert_ne!(second, first);
    }

    #[test]
    fn decline_reasons_are_case_insensitive_and_decide_the_block() {
        // Lichess sends lower-case keys in the event stream ("nobot"); the API docs say "noBot".
        for k in ["nobot", "noBot", "NOBOT", "casual", "rated", "variant", "timeControl", "timecontrol", "tooFast", "toofast", "onlyBot", "onlybot", "standard"] {
            assert_eq!(block_duration(k), BLOCK_LONG, "{k}");
        }
        assert_eq!(block_duration("generic"), BLOCK_MEDIUM);
        assert_eq!(block_duration("later"), BLOCK_SHORT);
        assert_eq!(block_duration("something-new"), BLOCK_SHORT);

        let mut c = challenger(3);
        let t0 = Instant::now();
        c.on_sent("a", "alpha", t0);
        assert_eq!(c.on_declined("zzz", "nobot", t0), None, "someone else's challenge");
        assert_eq!(c.on_declined("a", "nobot", t0), Some("alpha".into()));
        assert!(c.is_blocked("ALPHA", t0 + Duration::from_secs(20 * 3600)), "a lower-case nobot must be a long block");
    }

    #[test]
    fn per_opponent_daily_limit_blocks_exactly_as_long_as_lichess_says() {
        let mut c = challenger(4);
        let t0 = Instant::now();
        // "X played 100 games against other bots today, please wait ..." => HTTP 400 + seconds
        c.on_send_failed("lynx_bot", Some(400), Some(19_662), t0);
        assert!(c.is_blocked("lynx_bot", t0 + Duration::from_secs(19_000)), "still limited after 5 hours");
        assert!(!c.is_blocked("lynx_bot", t0 + Duration::from_secs(19_662 + 60)), "free again afterwards");
        // a block never exceeds a day
        c.on_send_failed("other", Some(400), Some(10 * 24 * 3600), t0);
        assert!(!c.is_blocked("other", t0 + BLOCK_LONG + Duration::from_secs(1)));
        // no wait given => short block
        c.on_send_failed("third", Some(500), None, t0);
        assert!(c.is_blocked("third", t0 + Duration::from_secs(60)));
        assert!(!c.is_blocked("third", t0 + BLOCK_SHORT + Duration::from_secs(1)));
    }

    #[test]
    fn a_failed_attempt_retries_after_seconds_not_after_the_whole_interval() {
        let mut c = challenger(5);
        let t0 = Instant::now();
        let cands = names(&["a", "b", "c"]);
        let first = send_target(c.next_action(t0, true, &cands));
        c.on_send_failed(&first, Some(400), Some(3600), t0);
        assert_eq!(c.next_action(t0 + Duration::from_secs(2), true, &cands), Action::Wait);
        let second = send_target(c.next_action(t0 + FAILED_RETRY_GAP + Duration::from_secs(1), true, &cands));
        assert_ne!(second, first);
    }

    #[test]
    fn long_failure_streak_pauses_everything() {
        let mut c = challenger(6);
        let cands: Vec<String> = (0..50).map(|i| format!("bot{i}")).collect();
        let mut now = Instant::now();
        for _ in 0..FAILURE_STREAK {
            now += Duration::from_secs(10);
            let n = send_target(c.next_action(now, true, &cands));
            c.on_send_failed(&n, Some(400), Some(7200), now);
        }
        assert_eq!(c.next_action(now + Duration::from_secs(60), true, &cands), Action::Wait, "paused");
        assert!(matches!(c.next_action(now + FAILURE_STREAK_PAUSE + Duration::from_secs(20), true, &cands), Action::Send(_)));
    }

    #[test]
    fn hourly_quota_and_rate_limit_pause() {
        let mut c = challenger(8);
        let cands = names(&["a", "b", "c", "d"]);
        let t0 = Instant::now();
        for i in 0..3u64 {
            let t = t0 + Duration::from_secs(40 * i);
            let n = send_target(c.next_action(t, true, &cands));
            c.on_sent(&format!("id{i}"), &n, t);
            c.on_resolved(&format!("id{i}"));
        }
        let t = t0 + Duration::from_secs(40 * 3);
        assert_eq!(c.next_action(t, true, &cands), Action::Wait, "3 per hour reached");
        let later = t0 + HOUR + Duration::from_secs(1);
        assert!(matches!(c.next_action(later, true, &cands), Action::Send(_)), "quota frees up after an hour");
        c.on_send_failed("a", Some(429), None, later);
        assert_eq!(c.next_action(later + Duration::from_secs(120), true, &cands), Action::Wait);
        assert!(matches!(c.next_action(later + RATE_LIMIT_PAUSE + Duration::from_secs(40), true, &cands), Action::Send(_)));
    }

    #[test]
    fn availability_counts_only_usable_opponents() {
        let mut c = challenger(9);
        let t0 = Instant::now();
        let cands = names(&["mybot", "a", "b", "c"]);
        assert_eq!(c.available(&cands, t0), 3);
        c.on_send_failed("a", Some(400), Some(3600), t0);
        assert_eq!(c.available(&cands, t0), 2);
        assert_eq!(c.available(&[], t0), 0);
    }

    #[test]
    fn no_candidates_means_wait() {
        let mut c = challenger(1);
        assert_eq!(c.next_action(Instant::now(), true, &[]), Action::Wait);
        assert_eq!(c.next_action(Instant::now(), true, &names(&["mybot"])), Action::Wait);
    }

    #[test]
    fn clock_parsing() {
        assert_eq!(parse_clock("180+2"), Ok((180, 2)));
        assert_eq!(parse_clock(" 600 + 0 "), Ok((600, 0)));
        assert_eq!(parse_clock("0+5"), Ok((0, 5)));
        assert!(parse_clock("180").is_err());
        assert!(parse_clock("a+b").is_err());
        assert!(parse_clock("0+0").is_err());
        assert!(parse_clock("20000+0").is_err());
        assert!(parse_clock("60+999").is_err());
    }
}
