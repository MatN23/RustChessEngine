//! Tests for the Rust Lichess bot.
//!
//! * unit tests: time allocation (differential vs the Python original), challenge
//!   policy, slot bookkeeping, NDJSON parsing, input validation, config parsing
//! * pondering stress test (real engine)
//! * end-to-end games against the in-process mock Lichess server, with the real
//!   engine on both sides.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::bot::{decide_challenge, Bot, BotStats, Decision, Games};
use super::client::{ApiError, LichessClient, NdjsonStream, StreamItem};
use super::game::Pondering;
use super::mock::{challenge_event, MockOptions, MockServer, TOKEN};
use super::timing::{budget_for_turn, calculate_time_allocation, move_time_ms};
use super::timing_cases::CASES;
use super::Config;
use crate::board::BoardState;
use crate::search::SearchEngine;

// ---------------------------------------------------------------------------
// time management
// ---------------------------------------------------------------------------

#[test]
fn time_allocation_matches_python_original_exactly() {
    for &(t, inc, n, py_raw, py_final) in CASES {
        let raw = calculate_time_allocation(t, inc, n);
        assert!(
            (raw - py_raw).abs() < 1e-9,
            "raw mismatch for time={t} inc={inc} ply={n}: rust {raw} vs python {py_raw}"
        );

        // The Rust budget equals what Python used, except where the new
        // safety cap deliberately lowers it.
        let cap = (t.saturating_sub(300) / 2).max(50);
        assert_eq!(
            move_time_ms(t, inc, n, 300),
            py_final.min(cap),
            "budget mismatch for time={t} inc={inc} ply={n}"
        );
    }
}

#[test]
fn time_budget_never_flags_the_bot() {
    // Whenever there is more clock than the overhead, search + overhead must fit.
    for t in (0..=600_000u64).step_by(250) {
        for inc in [0u64, 1000, 5000] {
            for ply in [0usize, 10, 30, 80] {
                let ms = move_time_ms(t, inc, ply, 300);
                if t > 400 {
                    assert!(ms + 300 <= t, "t={t} inc={inc} ply={ply}: budget {ms}ms");
                }
                assert!(ms >= 50);
            }
        }
    }
    // The case the Python version got wrong: 400 ms on the clock.
    assert!(move_time_ms(400, 0, 20, 300) <= 50);
}

#[test]
fn clockless_game_budget_is_opt_in() {
    // Default (setting unset): legacy behaviour, a missing clock is treated as 60 s => <= ~1.7 s.
    let legacy = budget_for_turn(None, 0, 20, 300, None);
    assert_eq!(legacy, move_time_ms(60_000, 0, 20, 300));
    assert!(legacy <= 1700);
    // UNLIMITED_MOVE_TIME_MS set: exactly that think time, at any point of the game.
    assert_eq!(budget_for_turn(None, 0, 20, 300, Some(30_000)), 30_000);
    assert_eq!(budget_for_turn(None, 0, 0, 300, Some(45_000)), 45_000);
    // A real clock is never affected by the setting.
    for unl in [None, Some(30_000)] {
        assert_eq!(
            budget_for_turn(Some(180_000), 2000, 20, 300, unl),
            move_time_ms(180_000, 2000, 20, 300)
        );
    }
}

// ---------------------------------------------------------------------------
// challenge policy + slots
// ---------------------------------------------------------------------------

fn ch(variant: &str, speed: &str, tc: &str) -> serde_json::Value {
    challenge_event("abc", variant, speed, tc)["challenge"].clone()
}

#[test]
fn challenge_policy() {
    assert_eq!(decide_challenge(&ch("standard", "blitz", "clock"), true), Decision::Accept);
    assert_eq!(decide_challenge(&ch("fromPosition", "rapid", "clock"), true), Decision::Accept);
    assert_eq!(decide_challenge(&ch("crazyhouse", "blitz", "clock"), true), Decision::Decline("variant"));
    assert_eq!(decide_challenge(&ch("chess960", "blitz", "clock"), true), Decision::Decline("variant"));
    assert_eq!(decide_challenge(&ch("standard", "correspondence", "correspondence"), true), Decision::Decline("timeControl"));
    assert_eq!(decide_challenge(&ch("standard", "correspondence", "unlimited"), true), Decision::Decline("timeControl"));
    assert_eq!(decide_challenge(&ch("standard", "blitz", "clock"), false), Decision::Decline("later"));
    // variant is checked before capacity, so a variant never gets "later"
    assert_eq!(decide_challenge(&ch("atomic", "blitz", "clock"), false), Decision::Decline("variant"));
    // missing fields default to standard / clock
    assert_eq!(decide_challenge(&json!({"id": "x"}), true), Decision::Accept);
}

#[test]
fn game_slot_bookkeeping() {
    let mut g = Games::default();
    assert!(g.try_reserve("a", 1));
    assert!(!g.try_reserve("b", 1), "second challenge must be refused while one is reserved");
    assert!(g.try_reserve("a", 1), "re-reserving the same id is idempotent");
    assert!(g.start_playing("a"));
    assert!(!g.start_playing("a"), "duplicate gameStart must not spawn a second thread");
    assert_eq!(g.count(), 1);
    assert!(!g.try_reserve("b", 1));
    g.finish("a");
    assert_eq!(g.count(), 0);
    assert!(g.try_reserve("b", 1));
    g.release_reservation("b");
    assert_eq!(g.count(), 0);
    // a game that started without a reservation (replayed on reconnect) still counts
    assert!(g.start_playing("c"));
    assert!(!g.try_reserve("d", 1));
}

// ---------------------------------------------------------------------------
// NDJSON + validation + config
// ---------------------------------------------------------------------------

fn collect(mut s: NdjsonStream) -> Vec<Option<serde_json::Value>> {
    let mut out = vec![];
    while let Some(item) = s.next_item() {
        match item.expect("no io error") {
            StreamItem::Event(v) => out.push(Some(v)),
            StreamItem::KeepAlive => out.push(None),
        }
    }
    out
}

#[test]
fn ndjson_parsing() {
    let data = b"{\"type\":\"gameStart\"}\n\n   \nnot json at all\n{\"type\":\"challenge\",\"n\":1}\r\n{\"type\":\"last\"}".to_vec();
    let items = collect(NdjsonStream::from_reader(Cursor::new(data)));
    assert_eq!(items.len(), 6);
    assert_eq!(items[0].as_ref().unwrap()["type"], "gameStart");
    assert!(items[1].is_none(), "blank line is a keep-alive");
    assert!(items[2].is_none(), "whitespace line is a keep-alive");
    assert!(items[3].is_none(), "garbage is skipped, not fatal");
    assert_eq!(items[4].as_ref().unwrap()["n"], 1, "CRLF handled");
    assert_eq!(items[5].as_ref().unwrap()["type"], "last", "final line without newline is delivered");
    let items = collect(NdjsonStream::from_reader(Cursor::new(b"{\"a\":1}\n{\"b\":2}".to_vec())));
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].as_ref().unwrap()["b"], 2);
}

#[test]
fn client_refuses_malformed_ids_and_moves() {
    // Never touches the network: validation happens first.
    let c = LichessClient::new("http://127.0.0.1:9", "t");
    for bad in ["", "../etc", "ab/cd", "a b", "gameé", &"x".repeat(40)] {
        assert!(matches!(c.make_move(bad, "e2e4"), Err(ApiError::BadInput(_))), "{bad:?}");
        assert!(matches!(c.open_game_stream(bad), Err(ApiError::BadInput(_))), "{bad:?}");
        assert!(matches!(c.accept_challenge(bad), Err(ApiError::BadInput(_))), "{bad:?}");
    }
    for bad in ["", "e2", "e2e", "e2e4e", "i2i4", "e9e4", "e2e4x", "e7e8k", "E2E4", "e2e4/../"] {
        assert!(matches!(c.make_move("abcd1234", bad), Err(ApiError::BadInput(_))), "{bad:?}");
    }
    // Well-formed input gets as far as the (refused) connection.
    assert!(matches!(c.make_move("abcd1234", "e7e8q"), Err(ApiError::Transport(_))));
}

#[test]
fn config_parsing() {
    for v in ["LICHESS_TOKEN", "LICHESS_URL", "ENGINE_THREADS", "ENGINE_HASH", "MOVE_OVERHEAD", "SEARCH_DEPTH", "PONDER", "PONDER_MAX_MS", "MAX_GAMES"] {
        std::env::remove_var(v);
    }
    assert!(Config::from_env(&[]).is_err(), "no token");

    let c = Config::from_env(&["tok".to_string()]).unwrap();
    assert_eq!(c.token, "tok");
    assert_eq!(c.base_url, "https://lichess.org");
    // same defaults as the Python bot
    assert_eq!((c.threads, c.hash_mb, c.move_overhead_ms), (8, 2048, 300));
    assert!(c.ponder);
    assert_eq!(c.max_games, 1);

    std::env::set_var("LICHESS_TOKEN", "envtok");
    std::env::set_var("LICHESS_URL", "http://localhost:1234/");
    std::env::set_var("ENGINE_THREADS", "2");
    std::env::set_var("PONDER", "0");
    let c = Config::from_env(&["argtok".to_string()]).unwrap();
    assert_eq!(c.token, "envtok", "env wins over argv like the Python bot");
    assert_eq!(c.base_url, "http://localhost:1234", "trailing slash stripped");
    assert_eq!(c.threads, 2);
    assert!(!c.ponder);

    std::env::set_var("ENGINE_THREADS", "lots");
    assert!(Config::from_env(&[]).is_err(), "typo'd number is an error, not a silent default");
    std::env::set_var("ENGINE_THREADS", "0");
    assert!(Config::from_env(&[]).is_err());
    std::env::set_var("ENGINE_THREADS", "2");
    std::env::set_var("LICHESS_URL", "ftp://x");
    assert!(Config::from_env(&[]).is_err());

    for v in ["LICHESS_TOKEN", "LICHESS_URL", "ENGINE_THREADS", "PONDER"] {
        std::env::remove_var(v);
    }
}

// ---------------------------------------------------------------------------
// pondering
// ---------------------------------------------------------------------------

/// Engines allocate a 512 MB table on creation; run the heavy tests one at a
/// time so the suite stays within a few hundred MB regardless of test threads.
static HEAVY: Mutex<()> = Mutex::new(());

fn small_engine() -> Arc<SearchEngine> {
    let e = Arc::new(SearchEngine::new(1));
    e.set_hash_size(16);
    e
}

#[test]
fn pondering_stop_is_prompt_and_never_blocks_the_next_search() {
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());
    let engine = small_engine();
    // 60 s ponder budget: if a stop is ever lost, the follow-up search below
    // would sit behind it for a minute and this test would fail loudly.
    let mut ponder = Pondering::new(Arc::clone(&engine), 60_000);

    let mut after_e4 = BoardState::default();
    after_e4.make_move_uci("e2e4").unwrap();

    // Delays chosen to land inside the predict step, at the predict->main
    // transition, and inside the long search.
    let delays_ms = [0u64, 0, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 150, 250, 400];
    for (i, d) in delays_ms.iter().enumerate() {
        ponder.start(after_e4.clone());
        thread::sleep(Duration::from_millis(*d));

        let t = Instant::now();
        ponder.stop();
        let stop_took = t.elapsed();
        assert!(stop_took < Duration::from_secs(2), "round {i}: stop() took {stop_took:?}");
        assert!(!ponder.is_active());

        let t = Instant::now();
        let res = engine.search(after_e4.clone(), 3, Some(300));
        assert!(res.best_move.is_some());
        assert!(t.elapsed() < Duration::from_secs(3), "round {i}: search blocked {:?}", t.elapsed());
    }
}

#[test]
fn pondering_stop_survives_the_stop_flag_reset_race() {
    // Deterministic version of the race: the ponder thread is parked *after* its last
    // generation check and *before* engine.search(), which clears the stop flag on
    // entry. A single engine.stop() + join (the naive approach) is lost here and join()
    // would wait for the whole 60 s ponder budget.
    use super::game::PONDER_PRE_SEARCH_DELAY_MS;
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());
    let engine = small_engine();
    let mut ponder = Pondering::new(Arc::clone(&engine), 60_000);
    let mut after_e4 = BoardState::default();
    after_e4.make_move_uci("e2e4").unwrap();

    PONDER_PRE_SEARCH_DELAY_MS.store(400, Ordering::SeqCst);
    ponder.start(after_e4.clone());
    thread::sleep(Duration::from_millis(100)); // worker is now inside the window

    let t = Instant::now();
    ponder.stop();
    let took = t.elapsed();
    PONDER_PRE_SEARCH_DELAY_MS.store(0, Ordering::SeqCst);

    assert!(took < Duration::from_secs(5), "stop() took {took:?}: the stop signal was lost");
    assert!(took >= Duration::from_millis(250), "hook did not engage ({took:?}); test is not exercising the race");
    let t = Instant::now();
    assert!(engine.search(after_e4, 3, Some(300)).best_move.is_some());
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn pondering_drop_stops_the_thread() {
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());
    let engine = small_engine();
    {
        let mut p = Pondering::new(Arc::clone(&engine), 60_000);
        p.start(BoardState::default());
        thread::sleep(Duration::from_millis(50));
    } // dropped here
    let t = Instant::now();
    engine.search(BoardState::default(), 2, Some(200));
    assert!(t.elapsed() < Duration::from_secs(2));
}

// ---------------------------------------------------------------------------
// end-to-end against the mock server
// ---------------------------------------------------------------------------

struct Outcome {
    bot_moves: Vec<String>,
    rejected: Vec<String>,
    accepted: Vec<String>,
    declined: Vec<(String, String)>,
    status: String,
    plies: usize,
    streams_opened: usize,
    unauthorized: usize,
    finished: usize,
    abandoned: usize,
    panics: usize,
}

fn test_config(url: &str) -> Config {
    Config {
        token: TOKEN.into(),
        base_url: url.into(),
        threads: 1,
        hash_mb: 16,
        move_overhead_ms: 50,
        unlimited_move_ms: None,
        full_budget: false,
        search_depth: 32,
        ponder: true,
        ponder_max_ms: 5_000,
        max_games: 1,
        auto_challenge: false,
        challenge_bots: vec![],
        challenge_clock: (180, 2),
        challenge_rated: false,
        challenge_interval_secs: 5,
        challenge_max_per_hour: 10,
    }
}

fn base_opts() -> MockOptions {
    MockOptions {
        challenges: vec![challenge_event("game0001", "standard", "blitz", "clock")],
        // Small clock => ~200-500 ms searches; the mock never decrements it.
        clock_ms: 6_000,
        ..Default::default()
    }
}

/// Run the real `Bot` against the mock until `done` is true (or time runs out).
fn run_scenario(
    opts: MockOptions,
    tweak: impl FnOnce(&mut Config),
    done: impl Fn(&MockServer, &BotStats) -> bool,
) -> Outcome {
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::start(opts);
    let mut cfg = test_config(&server.url);
    tweak(&mut cfg);
    let bot = Bot::connect(cfg).expect("connect to mock");
    let stats = bot.stats();

    let shutdown = Arc::new(AtomicBool::new(false));
    let sd = Arc::clone(&shutdown);
    let runner = thread::spawn(move || bot.run(&sd));

    let deadline = Instant::now() + Duration::from_secs(150);
    while !done(&server, &stats) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let timed_out = !done(&server, &stats);

    shutdown.store(true, Ordering::SeqCst);
    let run_result = runner.join().expect("bot run thread must not panic");
    assert!(run_result.is_ok(), "bot.run returned {run_result:?}");
    assert!(!timed_out, "scenario timed out");

    let st = server.state.lock().unwrap();
    let (status, plies) = st
        .game
        .as_ref()
        .map(|g| (g.status.clone(), g.moves.len()))
        .unwrap_or_default();
    Outcome {
        bot_moves: st.bot_moves.clone(),
        rejected: st.rejected.clone(),
        accepted: st.accepted.clone(),
        declined: st.declined.clone(),
        status,
        plies,
        streams_opened: st.game_streams_opened,
        unauthorized: st.unauthorized,
        finished: stats.finished.load(Ordering::SeqCst),
        abandoned: stats.abandoned.load(Ordering::SeqCst),
        panics: stats.panics.load(Ordering::SeqCst),
    }
}

/// Mock game over AND the bot's game thread has wound down.
fn game_done(srv: &MockServer, st: &BotStats) -> bool {
    srv.game_finished() && st.finished.load(Ordering::SeqCst) + st.abandoned.load(Ordering::SeqCst) + st.panics.load(Ordering::SeqCst) >= 1
}

fn assert_clean(o: &Outcome) {
    assert!(o.rejected.is_empty(), "mock rejected bot moves: {:?}", o.rejected);
    assert_eq!(o.panics, 0, "game thread panicked");
    assert_eq!(o.abandoned, 0, "bot abandoned the game");
    assert_eq!(o.finished, 1, "bot did not see the game finish");
    assert_ne!(o.status, "started");
    assert_eq!(o.unauthorized, 0);
}

#[test]
fn e2e_full_game_as_white_ignores_duplicate_states() {
    let o = run_scenario(
        MockOptions {
            bot_white: true,
            max_plies: 24,
            opp_think_ms: 120, // lets pondering actually run between moves
            duplicate_states: true,
            ..base_opts()
        },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    assert_eq!(o.accepted, vec!["game0001"]);
    assert!(o.declined.is_empty());
    // White moves on plies 1,3,5...; a drawn 24-ply game means exactly 12 bot moves.
    // (Duplicate gameStates must not produce extra moves: those would be rejected.)
    if o.status == "draw" {
        assert_eq!(o.plies, 24);
        assert_eq!(o.bot_moves.len(), 12);
    }
    assert!(o.bot_moves.len() >= 6, "only {} moves played", o.bot_moves.len());
}

#[test]
fn e2e_full_game_as_black_opponent_moves_first() {
    let o = run_scenario(
        MockOptions { bot_white: false, max_plies: 22, opp_think_ms: 60, ..base_opts() },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    if o.status == "draw" {
        assert_eq!(o.plies, 22);
        assert_eq!(o.bot_moves.len(), 11);
    }
    assert!(o.bot_moves.len() >= 5);
}

#[test]
fn e2e_custom_start_position_kiwipete() {
    // The old Python bot rebuilt every position from the standard start and ignored
    // initialFen. Kiwipete has castling on both wings and many tactics.
    let o = run_scenario(
        MockOptions {
            initial_fen: Some("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1".into()),
            bot_white: true,
            max_plies: 20,
            ..base_opts()
        },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    assert!(o.bot_moves.len() >= 5);
}

#[test]
fn e2e_custom_fen_where_black_moves_first() {
    // Bot is black and it is black's turn at 0 moves. Parity-based turn detection
    // ("even number of moves = white's turn") gets this wrong.
    let o = run_scenario(
        MockOptions {
            initial_fen: Some("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1".into()),
            bot_white: false,
            max_plies: 16,
            ..base_opts()
        },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    assert!(o.bot_moves.len() >= 4);
}

#[test]
fn e2e_threefold_repetition_in_history_does_not_stall_the_bot() {
    // Lichess does not end the game at threefold (it is only claimable), so the bot must
    // keep moving. `BoardState::is_game_over()` is true here; using it to decide
    // "nothing to do" would make the bot sit there until its flag fell.
    let preset: Vec<String> = "g1f3 g8f6 f3g1 f6g8 g1f3 g8f6 f3g1 f6g8".split(' ').map(String::from).collect();
    let mut b = BoardState::default();
    for m in &preset {
        b.make_move_uci(m).unwrap();
    }
    assert!(b.is_game_over(), "precondition: engine considers this position drawn");

    let o = run_scenario(
        MockOptions { preset_moves: preset, bot_white: true, max_plies: 12, ..base_opts() },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    assert_eq!(o.bot_moves.len(), 2, "bot must move at plies 9 and 11");
    assert_eq!(o.plies, 12);
}

#[test]
fn e2e_challenge_policy_over_the_wire() {
    let o = run_scenario(
        MockOptions {
            max_plies: 8,
            challenges: vec![
                challenge_event("crazy001", "crazyhouse", "blitz", "clock"),
                challenge_event("corr0001", "standard", "correspondence", "correspondence"),
                challenge_event("game0001", "standard", "blitz", "clock"),
                challenge_event("game0002", "standard", "blitz", "clock"), // slot already taken
            ],
            ..base_opts()
        },
        |_| {},
        |srv, st| game_done(srv, st) && srv.state.lock().unwrap().declined.len() >= 3,
    );
    assert_clean(&o);
    assert_eq!(o.accepted, vec!["game0001"]);
    let mut declined = o.declined.clone();
    declined.sort();
    assert_eq!(
        declined,
        vec![
            ("corr0001".to_string(), "timeControl".to_string()),
            ("crazy001".to_string(), "variant".to_string()),
            ("game0002".to_string(), "later".to_string()),
        ]
    );
}

#[test]
fn e2e_game_stream_drop_mid_game_reconnects_and_continues() {
    // Stream closes right after the bot's 3rd move while its ponder search is
    // running; the bot must reconnect, stop that ponder search, rebuild the
    // position from gameFull, and carry on without re-sending or skipping moves.
    let o = run_scenario(
        MockOptions {
            bot_white: true,
            max_plies: 20,
            opp_think_ms: 100,
            drop_stream_after_bot_move: Some(3),
            ..base_opts()
        },
        |_| {},
        game_done,
    );
    assert_clean(&o);
    assert!(o.streams_opened >= 2, "expected a reconnect, saw {} stream(s)", o.streams_opened);
    if o.status == "draw" {
        assert_eq!(o.bot_moves.len(), 10);
    }
}

#[test]
fn e2e_without_pondering_also_plays_cleanly() {
    let o = run_scenario(
        MockOptions { max_plies: 12, ..base_opts() },
        |c| c.ponder = false,
        game_done,
    );
    assert_clean(&o);
    assert!(o.bot_moves.len() >= 3);
}

#[test]
fn wrong_token_fails_fast() {
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());
    let server = MockServer::start(base_opts());
    let mut cfg = test_config(&server.url);
    cfg.token = "not-the-token".into();
    let err = Bot::connect(cfg).err().expect("must fail");
    assert!(err.contains("401"), "error should mention the HTTP status: {err}");
    assert_eq!(server.state.lock().unwrap().unauthorized, 1);
}

// ---------------------------------------------------------------------------
// the real binary, as a separate process, against the mock
// ---------------------------------------------------------------------------

#[test]
fn e2e_real_binary_process() {
    use std::process::{Command, Stdio};

    // target/<profile>/deps/<test-exe>  ->  target/<profile>/lichess_bot
    let exe = std::env::current_exe().unwrap();
    let bin = exe.parent().unwrap().parent().unwrap().join("lichess_bot");
    if !bin.exists() {
        eprintln!("SKIP: {} not built (run `cargo build --bin lichess_bot` first)", bin.display());
        return;
    }
    let _g = HEAVY.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::start(MockOptions { max_plies: 10, ..base_opts() });
    let mut child = Command::new(&bin)
        .env("LICHESS_TOKEN", TOKEN)
        .env("LICHESS_URL", &server.url)
        .env("ENGINE_THREADS", "1")
        .env("ENGINE_HASH", "16")
        .env("MOVE_OVERHEAD", "50")
        .env("PONDER_MAX_MS", "3000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lichess_bot");

    let deadline = Instant::now() + Duration::from_secs(120);
    while !server.game_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(Duration::from_millis(500));
    let finished = server.game_finished();
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(finished, "game did not finish.\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Logged in as TestBot"), "{stdout}");
    assert!(stdout.contains("playing as WHITE"), "{stdout}");
    assert!(stdout.contains("game finished: draw"), "{stdout}");
    assert!(!stderr.contains("PANICKED") && !stderr.contains("rejected"), "{stderr}");
    assert!(!stdout.contains(TOKEN) && !stderr.contains(TOKEN), "token must never be logged");
    let st = server.state.lock().unwrap();
    assert!(st.rejected.is_empty(), "{:?}", st.rejected);
    assert_eq!(st.bot_moves.len(), 5);
}

// ---------------------------------------------------------------------------
// auto-challenge (opt-in)
// ---------------------------------------------------------------------------

#[test]
fn challenge_client_calls_hit_the_right_endpoints_with_the_right_parameters() {
    let server = MockServer::start(MockOptions {
        online_bots: vec!["MaiaLike".into(), "Other-Bot".into(), "bad name!".into()],
        ..Default::default()
    });
    let client = LichessClient::new(&server.url, TOKEN);

    let id = client.create_challenge("SomeBot", false, 180, 2).expect("create");
    assert_eq!(id, "mockch01");
    client.cancel_challenge(&id).expect("cancel");
    // invalid usernames are refused before any request is made
    assert!(matches!(client.create_challenge("bad name!", false, 180, 2), Err(ApiError::BadInput(_))));
    assert!(matches!(client.create_challenge("../x", false, 180, 2), Err(ApiError::BadInput(_))));

    let bots = client.online_bots(50).expect("online bots");
    assert_eq!(bots, vec!["MaiaLike".to_string(), "Other-Bot".to_string()], "invalid names are filtered out");

    let st = server.state.lock().unwrap();
    assert_eq!(st.created.len(), 1);
    let (user, body) = &st.created[0];
    assert_eq!(user, "SomeBot");
    for part in ["rated=false", "clock.limit=180", "clock.increment=2", "variant=standard"] {
        assert!(body.contains(part), "missing {part} in {body}");
    }
    assert_eq!(st.cancelled, vec!["mockch01".to_string()]);
}

#[test]
fn per_opponent_limit_error_exposes_the_wait_time() {
    // exactly what Lichess returned in the log: HTTP 400 with ratelimit.seconds
    let body = r#"{"error":"Lynx_BOT played 100 games against other bots today, please wait until 2026-11-10T08:08:19.748Z to challenge them.","ratelimit":{"key":"bot.vsBot.day","seconds":19662}}"#;
    let e = ApiError::Status(400, body.to_string());
    assert_eq!(e.rate_limit_seconds(), Some(19_662));
    // truncated body (the client keeps 300 bytes) still yields the number
    let cut = ApiError::Status(400, body[..body.len() - 2].to_string()); // closing braces lost
    assert_eq!(cut.rate_limit_seconds(), Some(19_662));
    assert_eq!(ApiError::Status(500, "boom".into()).rate_limit_seconds(), None);
    assert_eq!(ApiError::Transport("x".into()).rate_limit_seconds(), None);
}

#[test]
fn rate_limit_on_challenge_creation_surfaces_as_status_429() {
    let server = MockServer::start(MockOptions { create_status: 429, ..Default::default() });
    let client = LichessClient::new(&server.url, TOKEN);
    assert!(matches!(client.create_challenge("SomeBot", false, 180, 2), Err(ApiError::Status(429, _))));
}

#[test]
fn bot_sends_a_challenge_when_enabled_and_never_when_disabled() {
    for enabled in [true, false] {
        let created = Arc::new(Mutex::new(Vec::new()));
        let cap = Arc::clone(&created);
        let waited_enough = Instant::now() + Duration::from_millis(2500);
        let opts = MockOptions { challenges: vec![], ..Default::default() };
        let _ = run_scenario(
            opts,
            move |cfg| {
                cfg.auto_challenge = enabled;
                cfg.challenge_bots = vec!["opponentbot".into()];
                cfg.challenge_clock = (300, 3);
                cfg.challenge_rated = true;
            },
            move |srv, _| {
                let st = srv.state.lock().unwrap();
                *cap.lock().unwrap() = st.created.clone();
                if enabled { !st.created.is_empty() } else { Instant::now() > waited_enough }
            },
        );
        let created = created.lock().unwrap();
        if enabled {
            assert_eq!(created.len(), 1, "exactly one challenge per interval");
            assert_eq!(created[0].0, "opponentbot");
            assert!(created[0].1.contains("clock.limit=300") && created[0].1.contains("rated=true"));
        } else {
            assert!(created.is_empty(), "feature is off by default: nothing must be sent");
        }
    }
}

#[test]
fn bot_discovers_online_bots_and_does_not_challenge_itself() {
    let created = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&created);
    let opts = MockOptions {
        challenges: vec![],
        online_bots: vec!["TestBot".into(), "RealOpponent".into()], // TestBot is the bot itself
        ..Default::default()
    };
    let _ = run_scenario(
        opts,
        |cfg| {
            cfg.auto_challenge = true;
            cfg.challenge_bots = vec![]; // discover
        },
        move |srv, _| {
            let st = srv.state.lock().unwrap();
            *cap.lock().unwrap() = st.created.clone();
            !st.created.is_empty()
        },
    );
    assert_eq!(created.lock().unwrap()[0].0, "RealOpponent");
}

#[test]
fn rate_limited_bot_backs_off_instead_of_hammering_lichess() {
    let attempts = Arc::new(Mutex::new(0usize));
    let cap = Arc::clone(&attempts);
    let until = Instant::now() + Duration::from_millis(3500);
    let opts = MockOptions { challenges: vec![], create_status: 429, ..Default::default() };
    let _ = run_scenario(
        opts,
        |cfg| {
            cfg.auto_challenge = true;
            cfg.challenge_bots = vec!["opponentbot".into()];
        },
        move |srv, _| {
            *cap.lock().unwrap() = srv.state.lock().unwrap().created.len();
            Instant::now() > until
        },
    );
    assert_eq!(*attempts.lock().unwrap(), 1, "after a 429 the bot must pause, not retry every tick");
}

#[test]
fn declined_challenge_blocks_the_opponent_and_frees_the_game_slot() {
    let server = MockServer::start(MockOptions::default());
    let mut cfg = test_config(&server.url);
    cfg.auto_challenge = true;
    let bot = Bot::connect(cfg).expect("connect");
    let ch = bot.challenger();
    let now = Instant::now();
    ch.lock().unwrap().on_sent("abcd1234", "maia9", now);
    assert!(bot.games().lock().unwrap().try_reserve("abcd1234", 1));
    assert_eq!(bot.games().lock().unwrap().count(), 1);

    bot.process_event(&json!({
        "type": "challengeDeclined",
        "challenge": {"id": "abcd1234", "declineReasonKey": "nobot"}
    }));
    assert!(!ch.lock().unwrap().has_pending());
    assert!(ch.lock().unwrap().is_blocked("maia9", Instant::now() + Duration::from_secs(3600)), "noBot is a long block");
    assert_eq!(bot.games().lock().unwrap().count(), 0, "slot released");

    // an accepted challenge becomes a game: gameStart clears the pending state
    ch.lock().unwrap().on_sent("efgh5678", "beta", now);
    bot.process_event(&json!({"type": "challengeCanceled", "challenge": {"id": "efgh5678"}}));
    assert!(!ch.lock().unwrap().has_pending());
}

#[test]
fn opponents_that_hit_lichess_daily_limit_are_skipped_not_retried() {
    // The real reply from the log: HTTP 400 + "ratelimit":{"seconds":19662}.
    let body = r#"{"error":"X played 100 games against other bots today, please wait until later to challenge them.","ratelimit":{"key":"bot.vsBot.day","seconds":19662}}"#;
    let created = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&created);
    let opts = MockOptions {
        challenges: vec![],
        online_bots: vec!["TestBot".into(), "alpha".into(), "beta".into(), "gamma".into(), "delta".into()],
        create_status: 400,
        create_body: body.to_string(),
        ..Default::default()
    };
    let _ = run_scenario(
        opts,
        |cfg| {
            cfg.auto_challenge = true;
            cfg.challenge_bots = vec![]; // discover from the online list
        },
        move |srv, _| {
            let st = srv.state.lock().unwrap();
            *cap.lock().unwrap() = st.created.clone();
            st.created.len() >= 3
        },
    );
    let created = created.lock().unwrap();
    let users: Vec<&str> = created.iter().map(|(u, _)| u.as_str()).collect();
    let unique: std::collections::HashSet<&&str> = users.iter().collect();
    assert_eq!(unique.len(), users.len(), "a rate-limited opponent was retried: {users:?}");
    assert!(!users.contains(&"TestBot"), "challenged itself: {users:?}");
}

#[test]
fn auto_challenge_settings_are_validated() {
    let server = MockServer::start(MockOptions::default());
    let mut cfg = test_config(&server.url);
    cfg.auto_challenge = true;
    assert!(cfg.validate().is_ok());
    cfg.challenge_interval_secs = 1;
    assert!(cfg.validate().is_err());
    cfg.challenge_interval_secs = 30;
    cfg.challenge_max_per_hour = 0;
    assert!(cfg.validate().is_err());
    cfg.challenge_max_per_hour = 5;
    cfg.challenge_bots = vec!["bad name!".into()];
    assert!(cfg.validate().is_err());
    // when the feature is off these settings are not checked
    cfg.auto_challenge = false;
    assert!(cfg.validate().is_ok());
}
