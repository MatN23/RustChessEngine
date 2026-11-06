//! Pure-Rust match runner: plays the engine (called in-process, exactly like the
//! Lichess bot does) against Stockfish configured like a Lichess AI level.
//!
//!   cargo run --release --no-default-features --bin sf_match -- \
//!       <level 1-8> <think_ms> <games> [stockfish_path] [threads] [hash_mb]
//!
//! Lichess AI levels (fishnet): skill [-9,-5,-1,3,7,11,16,20] (clamped to >=0 for
//! stock Stockfish), depth [5,5,5,5,5,8,13,22], movetime [50,100,150,200,300,400,500,1000] ms.

#![allow(dead_code, unused_imports, unused_variables)]

mod bitboard;
mod board;
mod eval;
mod movegen;
mod opening_book;
mod search;
mod zobrist;

use board::{BoardState, Color};
use movegen::MoveGenerator;
use search::SearchEngine;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

const SKILL: [i32; 8] = [-9, -5, -1, 3, 7, 11, 16, 20];
const DEPTH: [u32; 8] = [5, 5, 5, 5, 5, 8, 13, 22];
const MOVETIME: [u64; 8] = [50, 100, 150, 200, 300, 400, 500, 1000];

const OPENINGS: [&str; 8] = [
    "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6",
    "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6",
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4",
    "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7",
    "e2e4 e7e6 d2d4 d7d5 b1c3 g8f6",
    "c2c4 e7e5 b1c3 g8f6 g1f3 b8c6",
    "g1f3 d7d5 g2g3 g8f6 f1g2 e7e6",
    "e2e4 c7c6 d2d4 d7d5 e4e5 c8f5",
];

struct Uci {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Uci {
    fn spawn(path: &str) -> Uci {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cannot start opponent engine");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Uci { child, stdin, out }
    }
    fn send(&mut self, s: &str) {
        writeln!(self.stdin, "{}", s).unwrap();
        self.stdin.flush().unwrap();
    }
    fn wait_for(&mut self, prefix: &str) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let mut l = String::new();
            if self.out.read_line(&mut l).unwrap() == 0 {
                panic!("stockfish exited");
            }
            let l = l.trim().to_string();
            let done = l.starts_with(prefix);
            lines.push(l);
            if done {
                return lines;
            }
        }
    }
    fn setup(&mut self, opts: &[(&str, String)]) {
        self.send("uci");
        self.wait_for("uciok");
        for (k, v) in opts {
            self.send(&format!("setoption name {} value {}", k, v));
        }
        self.send("isready");
        self.wait_for("readyok");
    }
    fn position(&mut self, moves: &[String]) {
        if moves.is_empty() {
            self.send("position startpos");
        } else {
            self.send(&format!("position startpos moves {}", moves.join(" ")));
        }
    }
}

/// White-POV centipawn eval (mate -> +/-10000) from the judge at a fixed depth.
fn judge_eval(j: &mut Uci, moves: &[String], white_to_move: bool) -> i32 {
    j.position(moves);
    j.send("go depth 11");
    let lines = j.wait_for("bestmove");
    let mut score = 0;
    for l in lines.iter().rev() {
        if let Some(i) = l.find("score ") {
            let t: Vec<&str> = l[i..].split_whitespace().collect();
            if t.len() >= 3 {
                let v: i32 = t[2].parse().unwrap_or(0);
                score = if t[1] == "mate" {
                    if v > 0 { 10000 } else { -10000 }
                } else {
                    v
                };
            }
            break;
        }
    }
    if white_to_move { score } else { -score }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: sf_match <level 1-8> <think_ms> <games> [stockfish] [threads] [hash_mb]");
        std::process::exit(2);
    }
    let level: usize = a[1].parse().unwrap();
    let think_ms: u64 = a[2].parse().unwrap();
    let mut games: usize = a[3].parse().unwrap();
    // Colours must be balanced: every opening is played once with each colour.
    if games % 2 == 1 {
        games += 1;
        println!("note: odd game count rounded up to {} so both colours are tested equally", games);
    }
    // Adjudication is configurable and can be disabled (SF_NO_ADJ=1) for unbiased results.
    let adj_cp: i32 = std::env::var("SF_ADJ_CP").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    let adj_plies: i32 = std::env::var("SF_ADJ_PLIES").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let no_adj = std::env::var("SF_NO_ADJ").is_ok();
    let sf_path = a.get(4).cloned().unwrap_or("/usr/games/stockfish".into());
    let threads: usize = a.get(5).and_then(|v| v.parse().ok()).unwrap_or(1);
    let hash_mb: usize = a.get(6).and_then(|v| v.parse().ok()).unwrap_or(512);
    let versus_engine = level == 0; // level 0: opponent is another UCI engine (e.g. chess_uci build)
    let skill = if versus_engine { 0 } else { SKILL[level - 1].max(0) };
    let depth = if versus_engine { 64 } else { DEPTH[level - 1] };
    let sf_ms = if versus_engine { think_ms } else { MOVETIME[level - 1] };

    println!(
        "Lichess AI level {} => Skill {} depth {} movetime {}ms | engine: {} thread(s), {}MB hash, {}ms/move",
        level, skill, depth, sf_ms, threads, hash_mb, think_ms
    );

    let mut sf = if versus_engine { Uci::spawn(&sf_path) } else { Uci::spawn(&sf_path) };
    if versus_engine {
        sf.setup(&[("Threads", "1".into()), ("Hash", "64".into())]);
    } else {
        sf.setup(&[("Threads", "1".into()), ("Hash", "64".into()), ("Skill Level", skill.to_string())]);
    }
    let mut judge = Uci::spawn(&std::env::var("JUDGE_SF").unwrap_or("/usr/games/stockfish".into()));
    judge.setup(&[("Threads", "1".into()), ("Hash", "64".into())]);

    let engine = SearchEngine::new(threads);
    engine.set_hash_size(hash_mb);

    let (mut w, mut d, mut l) = (0u32, 0u32, 0u32);
    let (mut score_white, mut score_black, mut adjudicated) = (0.0f64, 0.0f64, 0u32);
    for g in 0..games {
        let engine_white = g % 2 == 0;
        let opening: Vec<String> = OPENINGS[(g / 2) % OPENINGS.len()].split(' ').map(String::from).collect();
        engine.new_game(); // clears the TT and all search state: nothing leaks between games
        sf.send("ucinewgame");
        sf.send("isready");
        sf.wait_for("readyok");
        let mut board = BoardState::default();
        let mut moves: Vec<String> = Vec::new();
        for m in &opening {
            board.make_move_uci(m).unwrap();
            moves.push(m.clone());
        }
        let t_game = Instant::now();
        let mut engine_time = 0.0f64;
        let mut depth_sum = 0u64;
        let mut n_eng_moves = 0u64;
        let mut adj_run = 0;
        let (result, reason): (f64, String) = loop {
            let legal = MoveGenerator::generate_legal_moves(&board);
            let white_to_move = board.side_to_move == Color::White;
            if legal.is_empty() {
                if board.is_in_check(board.side_to_move) {
                    let eng_moves = white_to_move != engine_white;
                    break (if eng_moves { 1.0 } else { 0.0 }, "checkmate".into());
                }
                break (0.5, "stalemate".into());
            }
            if board.halfmove_clock >= 100 { break (0.5, "50-move".into()); }
            if board.is_repetition() { break (0.5, "threefold".into()); }
            if board.is_insufficient_material() { break (0.5, "insufficient".into()); }
            if moves.len() >= 300 {
                let ev = judge_eval(&mut judge, &moves, white_to_move);
                let ev = if engine_white { ev } else { -ev };
                break (if ev >= 300 { 1.0 } else if ev <= -300 { 0.0 } else { 0.5 }, format!("ply cap, judge {:+}", ev));
            }
            // Adjudicate clearly decided games (judge +-900cp for 6 consecutive plies) to save time.
            if !no_adj && moves.len() > 40 {
                let ev = judge_eval(&mut judge, &moves, white_to_move);
                let ev_e = if engine_white { ev } else { -ev };
                if ev_e.abs() >= adj_cp { adj_run += 1 } else { adj_run = 0 }
                if adj_run >= adj_plies {
                    break (if ev_e > 0 { 1.0 } else { 0.0 }, format!("adjudicated {:+}", ev_e));
                }
            }

            let engine_turn = white_to_move == engine_white;
            let uci = if engine_turn {
                let t0 = Instant::now();
                let res = engine.search(board.clone(), 64, Some(think_ms));
                engine_time += t0.elapsed().as_secs_f64();
                n_eng_moves += 1;
                match res.best_move {
                    Some(m) if legal.contains(&m) => m.to_uci(),
                    _ => legal[0].to_uci(),
                }
            } else {
                sf.position(&moves);
                sf.send(&format!("go depth {} movetime {}", depth, sf_ms));
                let lines = sf.wait_for("bestmove");
                lines.last().unwrap().split_whitespace().nth(1).unwrap().to_string()
            };
            if board.make_move_uci(&uci).is_err() {
                // illegal move from either side
                break (if engine_turn { 0.0 } else { 1.0 }, format!("illegal move {}", uci));
            }
            moves.push(uci);
        };
        if result == 1.0 { w += 1 } else if result == 0.5 { d += 1 } else { l += 1 }
        if engine_white { score_white += result } else { score_black += result }
        if reason.starts_with("adjudicated") { adjudicated += 1; }
        println!(
            "game {:>2} engine={} plies={:>3} result={} ({}) engine_avg_move={:.1}s total={:.0}s | +{} ={} -{}",
            g + 1,
            if engine_white { "W" } else { "B" },
            moves.len(),
            result,
            reason,
            if n_eng_moves > 0 { engine_time / n_eng_moves as f64 } else { 0.0 },
            t_game.elapsed().as_secs_f64(),
            w, d, l
        );
        println!("   moves: {}", moves.join(" "));
        let _ = std::io::stdout().flush();
    }
    let n = (w + d + l) as f64;
    println!("FINAL level {}: +{} ={} -{} score={:.1}%", level, w, d, l, 100.0 * (w as f64 + d as f64 / 2.0) / n);
    println!("   as White: {:.1}/{}   as Black: {:.1}/{}   adjudicated games: {}", score_white, games / 2, score_black, games / 2, adjudicated);
    sf.send("quit");
    judge.send("quit");
}
