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
/// Opponents that were merely busy / unresponsive are retried later.
const BLOCK_SHORT: Duration = Duration::from_secs(15 * 60);
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
    next_index: usize,
    paused_until: Option<Instant>,
}

impl Challenger {
    pub fn new(settings: ChallengeSettings, my_id: &str) -> Self {
        Challenger {
            settings,
            my_id: my_id.to_lowercase(),
            blocked: HashMap::new(),
            pending: None,
            sent: VecDeque::new(),
            last_attempt: None,
            next_index: 0,
            paused_until: None,
        }
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
        let n = candidates.len();
        for k in 0..n {
            let name = &candidates[(self.next_index + k) % n];
            if name.eq_ignore_ascii_case(&self.my_id) || self.is_blocked(name, now) {
                continue;
            }
            self.next_index = (self.next_index + k + 1) % n;
            self.last_attempt = Some(now);
            return Action::Send(name.clone());
        }
        Action::Wait
    }

    /// The challenge was created (Lichess returned its id).
    pub fn on_sent(&mut self, id: &str, name: &str, now: Instant) {
        self.pending = Some(Pending {
            id: id.to_string(),
            name: name.to_string(),
            since: now,
        });
        self.sent.push_back(now);
    }

    /// Creating the challenge failed. `status` is the HTTP status if there was one.
    pub fn on_send_failed(&mut self, name: &str, status: Option<u16>, now: Instant) {
        if status == Some(429) {
            self.paused_until = Some(now + RATE_LIMIT_PAUSE);
        } else {
            self.block(name, now, BLOCK_SHORT);
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

/// How long to leave an opponent alone after it declined with Lichess's `declineReasonKey`.
pub fn block_duration(reason_key: &str) -> Duration {
    match reason_key {
        "noBot" | "onlyBot" | "standard" | "variant" | "rated" | "casual" | "timeControl"
        | "tooFast" | "tooSlow" => BLOCK_LONG,
        // "later", "generic" and anything unknown: try again after a while.
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

    #[test]
    fn round_robin_skips_self_and_blocked() {
        let mut c = Challenger::new(settings(), "MyBot");
        let t0 = Instant::now();
        let cands = names(&["mybot", "alpha", "beta"]);
        assert_eq!(c.next_action(t0, true, &cands), Action::Send("alpha".into()));
        c.on_sent("id1", "alpha", t0);
        assert!(c.on_declined("id1", "noBot", t0).is_some());
        // next attempt: alpha is blocked, self is skipped => beta
        let t1 = t0 + Duration::from_secs(31);
        assert_eq!(c.next_action(t1, true, &cands), Action::Send("beta".into()));
    }

    #[test]
    fn waits_for_slot_interval_and_pending() {
        let mut c = Challenger::new(settings(), "me");
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
        let mut c = Challenger::new(settings(), "me");
        let t0 = Instant::now();
        let cands = names(&["alpha", "beta"]);
        assert_eq!(c.next_action(t0, true, &cands), Action::Send("alpha".into()));
        c.on_sent("id1", "alpha", t0);
        assert_eq!(c.next_action(t0 + PENDING_TIMEOUT - Duration::from_secs(1), true, &cands), Action::Wait);
        let t1 = t0 + PENDING_TIMEOUT;
        assert_eq!(c.next_action(t1, true, &cands), Action::Cancel("id1".into()));
        c.on_timed_out("id1", t1);
        assert!(c.is_blocked("alpha", t1));
        assert_eq!(c.next_action(t1 + Duration::from_secs(31), true, &cands), Action::Send("beta".into()));
    }

    #[test]
    fn decline_reasons_decide_how_long_an_opponent_is_left_alone() {
        assert_eq!(block_duration("noBot"), BLOCK_LONG);
        assert_eq!(block_duration("timeControl"), BLOCK_LONG);
        assert_eq!(block_duration("rated"), BLOCK_LONG);
        assert_eq!(block_duration("later"), BLOCK_SHORT);
        assert_eq!(block_duration("generic"), BLOCK_SHORT);
        assert_eq!(block_duration("something-new"), BLOCK_SHORT);
        let mut c = Challenger::new(settings(), "me");
        let t0 = Instant::now();
        c.on_sent("a", "alpha", t0);
        assert_eq!(c.on_declined("zzz", "noBot", t0), None, "someone else's challenge");
        assert_eq!(c.on_declined("a", "later", t0), Some("alpha".into()));
        assert!(c.is_blocked("ALPHA", t0 + Duration::from_secs(60)), "case-insensitive");
        assert!(!c.is_blocked("alpha", t0 + BLOCK_SHORT + Duration::from_secs(1)), "short block expires");
    }

    #[test]
    fn hourly_quota_and_rate_limit_pause() {
        let mut c = Challenger::new(settings(), "me");
        let cands = names(&["a", "b", "c", "d"]);
        let t0 = Instant::now();
        for i in 0..3u64 {
            let t = t0 + Duration::from_secs(40 * i);
            let Action::Send(n) = c.next_action(t, true, &cands) else { panic!("expected a send") };
            c.on_sent(&format!("id{i}"), &n, t);
            c.on_resolved(&format!("id{i}"));
        }
        let t = t0 + Duration::from_secs(40 * 3);
        assert_eq!(c.next_action(t, true, &cands), Action::Wait, "3 per hour reached");
        let later = t0 + HOUR + Duration::from_secs(1);
        assert!(matches!(c.next_action(later, true, &cands), Action::Send(_)), "quota frees up after an hour");
        // 429 pauses everything for a while
        c.on_send_failed("a", Some(429), later);
        assert_eq!(c.next_action(later + Duration::from_secs(120), true, &cands), Action::Wait);
        assert!(matches!(c.next_action(later + RATE_LIMIT_PAUSE + Duration::from_secs(40), true, &cands), Action::Send(_)));
    }

    #[test]
    fn no_candidates_means_wait() {
        let mut c = Challenger::new(settings(), "me");
        assert_eq!(c.next_action(Instant::now(), true, &[]), Action::Wait);
        assert_eq!(c.next_action(Instant::now(), true, &names(&["me"])), Action::Wait);
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
