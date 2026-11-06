//! Replays a game logged by `sf_match` and reports the engine's biggest evaluation drops
//! (as judged by Stockfish), plus what the engine itself thinks of those positions.
//!   blunder_scan <log> <game_no> <W|B> [sf_depth] [engine_ms] [threshold_cp]
#![allow(dead_code, unused_imports, unused_variables)]
mod bitboard; mod board; mod eval; mod movegen; mod opening_book; mod search; mod zobrist;
use board::{BoardState, Color};
use eval::Evaluator;
use search::SearchEngine;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for i in 0..n {
        let mut p = i; for r in i + 1..n { if a[r][i].abs() > a[p][i].abs() { p = r; } }
        a.swap(i, p); b.swap(i, p);
        for r in i + 1..n { let f = a[r][i] / a[i][i]; for c in i..n { a[r][c] -= f * a[i][c]; } b[r] -= f * b[i]; }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() { let mut s = b[i]; for c in i + 1..n { s -= a[i][c] * x[c]; } x[i] = s / a[i][i]; }
    x
}

/// fit <step> <sf_depth> <logs...>: label natural midgame positions with Stockfish and
/// least-squares fit one weight per evaluation term.
fn fit(step: usize, depth: u32, logs: &[String]) {
    let mut sf = Command::new("/usr/games/stockfish").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut si = sf.stdin.take().unwrap();
    let mut so = BufReader::new(sf.stdout.take().unwrap());
    let mut send = |s: &str| { writeln!(si, "{}", s).unwrap(); si.flush().unwrap(); };
    let mut read_until = |p: &str| -> Vec<String> { let mut v = vec![]; loop { let mut l = String::new(); so.read_line(&mut l).unwrap(); let l = l.trim().to_string(); let d = l.starts_with(p); v.push(l); if d { return v; } } };
    send("uci"); read_until("uciok"); send("setoption name Hash value 128"); send("isready"); read_until("readyok");
    let mut rows: Vec<([i32; 11], f64, usize)> = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut game_id = 0usize;
    for path in logs {
        let text = std::fs::read_to_string(path).unwrap();
        for line in text.lines() {
            let l = line.trim().trim_start_matches("moves:").trim();
            let moves: Vec<String> = l.split_whitespace().map(String::from).collect();
            if moves.len() < 20 || !moves.iter().all(|m| m.len() >= 4 && m.len() <= 5 && m.as_bytes()[0].is_ascii_lowercase() && m.as_bytes()[1].is_ascii_digit()) { continue; }
            game_id += 1;
            let mut b = BoardState::default();
            for i in 0..moves.len() {
                if i >= 12 && i % step == 0 && b.all_pieces.count_ones() >= 14 && !b.is_in_check(b.side_to_move) && seen.insert(b.hash) {
                    send(&format!("position startpos moves {}", moves[..i].join(" "))); send(&format!("go depth {}", depth));
                    let lines = read_until("bestmove");
                    let mut cp = 0; for l in lines.iter().rev() { if let Some(k) = l.find("score ") { let t: Vec<&str> = l[k..].split_whitespace().collect();
                        let v: i32 = t[2].parse().unwrap_or(0); cp = if t[1] == "mate" { if v > 0 { 10000 } else { -10000 } } else { v }; break; } }
                    let white_cp = if b.side_to_move == Color::White { cp } else { -cp };
                    if white_cp.abs() <= 800 { rows.push((Evaluator::term_breakdown(&b), white_cp as f64, game_id)); }
                }
                b.make_move_uci(&moves[i]).unwrap();
            }
        }
    }
    println!("positions: {}", rows.len());
    let names = ["material+PST", "pawn_structure", "mobility", "king_safety", "space", "rook", "bishop", "knight", "tempo", "connectivity", "hanging"];
    let n = 11;
    // Cross-validation BY GAME: positions from one game are strongly correlated, so tuning and
    // validation sets must not share games (a per-position split leaks).
    const FOLDS: usize = 4;
    let games = game_id;
    println!("games: {}  folds: {} (fold = game_id % {})", games, FOLDS, FOLDS);
    let fit_w = |data: &Vec<&([i32; 11], f64, usize)>, active: &[bool]| -> Vec<f64> {
        let mut ata = vec![vec![0.0; n]; n]; let mut atb = vec![0.0; n];
        for (x, y, _) in data.iter() { for i in 0..n { atb[i] += x[i] as f64 * y; for j in 0..n { ata[i][j] += x[i] as f64 * x[j] as f64; } } }
        for i in 0..n { ata[i][i] += 50.0; if !active[i] { for j in 0..n { ata[i][j] = 0.0; ata[j][i] = 0.0; } ata[i][i] = 1.0; atb[i] = 0.0; } }
        solve(ata, atb)
    };
    let mae = |data: &Vec<&([i32; 11], f64, usize)>, w: &[f64]| -> f64 { data.iter().map(|(x, y, _)| ((0..n).map(|i| w[i] * x[i] as f64).sum::<f64>() - y).abs()).sum::<f64>() / data.len().max(1) as f64 };
    // Weights actually used by the engine (percent / 100), for comparison.
    let shipped = [1.0, 0.5, 1.0, 0.5, 0.5, 0.5, 1.0, 0.5, 1.0, 1.0, 1.0];
    let ones = vec![1.0; n];
    let (mut tot_ones, mut tot_ship, mut tot_fit, mut tot_n) = (0.0, 0.0, 0.0, 0.0);
    println!("fold  test_pos  MAE(old 1.0)  MAE(shipped 0.5)  MAE(fit on other folds)");
    for f in 0..FOLDS {
        let train: Vec<&([i32; 11], f64, usize)> = rows.iter().filter(|r| r.2 % FOLDS != f).collect();
        let test: Vec<&([i32; 11], f64, usize)> = rows.iter().filter(|r| r.2 % FOLDS == f).collect();
        if test.is_empty() { continue; }
        let mut active = vec![true; n];
        let mut w = fit_w(&train, &active);
        for _ in 0..n { let mut changed = false; for i in 1..n { if active[i] && w[i] < 0.0 { active[i] = false; changed = true; } } if !changed { break; } w = fit_w(&train, &active); }
        let (a1, a2, a3) = (mae(&test, &ones), mae(&test, &shipped), mae(&test, &w));
        println!("{:>4}  {:>8}  {:>12.1}  {:>16.1}  {:>23.1}", f, test.len(), a1, a2, a3);
        let t = test.len() as f64; tot_ones += a1 * t; tot_ship += a2 * t; tot_fit += a3 * t; tot_n += t;
    }
    println!("ALL   {:>8}  {:>12.1}  {:>16.1}  {:>23.1}", tot_n as usize, tot_ones / tot_n, tot_ship / tot_n, tot_fit / tot_n);
    println!("(lower is better; the 'shipped' weights were NOT fitted on the test fold's games only if they match the cross-validated fit pattern - compare the columns per fold)");
    // Standalone slope of each term on the residual of a material-only fit (all data)
    let all: Vec<&([i32; 11], f64, usize)> = rows.iter().collect();
    let names = ["material+PST", "pawn_structure", "mobility", "king_safety", "space", "rook", "bishop", "knight", "tempo", "connectivity", "hanging"];
    let mut a0 = vec![false; n]; a0[0] = true; let wm = fit_w(&all, &a0)[0];
    println!("standalone slope on residual of material-only fit (material weight {:.2}):", wm);
    for i in 1..n { let (mut sxr, mut sxx) = (0.0, 0.0); for (x, y, _) in all.iter() { let r = y - wm * x[0] as f64; sxr += x[i] as f64 * r; sxx += (x[i] as f64).powi(2); } println!("   {:<16} {:+.2}", names[i], sxr / sxx.max(1.0)); }
    send("quit");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "fit" { fit(a[2].parse().unwrap(), a[3].parse().unwrap(), &a[4..]); return; }
    let text = std::fs::read_to_string(&a[1]).unwrap();
    let game_no: usize = a[2].parse().unwrap();
    let engine_white = a[3] == "W";
    let sf_depth: u32 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(14);
    let eng_ms: u64 = a.get(5).and_then(|v| v.parse().ok()).unwrap_or(3000);
    let thr: i32 = a.get(6).and_then(|v| v.parse().ok()).unwrap_or(120);
    let line = text.lines().filter(|l| l.trim_start().starts_with("moves:")).nth(game_no - 1).expect("game not found");
    let moves: Vec<String> = line.trim_start().trim_start_matches("moves:").split_whitespace().map(String::from).collect();

    let mut sf = Command::new("/usr/games/stockfish").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut si = sf.stdin.take().unwrap();
    let mut so = BufReader::new(sf.stdout.take().unwrap());
    let mut send = |s: &str| { writeln!(si, "{}", s).unwrap(); si.flush().unwrap(); };
    let mut read_until = |p: &str| -> Vec<String> {
        let mut v = vec![]; loop { let mut l = String::new(); so.read_line(&mut l).unwrap(); let l = l.trim().to_string(); let d = l.starts_with(p); v.push(l); if d { return v; } } };
    send("uci"); read_until("uciok"); send("setoption name Hash value 128"); send("isready"); read_until("readyok");

    // returns (white-pov cp, best move)
    let mut analyse = |upto: usize| -> (i32, String) {
        if upto == 0 { send("position startpos"); } else { send(&format!("position startpos moves {}", moves[..upto].join(" "))); }
        send(&format!("go depth {}", sf_depth));
        let lines = read_until("bestmove");
        let best = lines.last().unwrap().split_whitespace().nth(1).unwrap().to_string();
        let mut cp = 0;
        for l in lines.iter().rev() { if let Some(i) = l.find("score ") { let t: Vec<&str> = l[i..].split_whitespace().collect();
            let v: i32 = t[2].parse().unwrap_or(0); cp = if t[1] == "mate" { if v > 0 { 10000 } else { -10000 } } else { v }; break; } }
        if upto % 2 == 1 { cp = -cp; } // side to move after an odd number of plies is black
        (cp, best)
    };

    let engine = SearchEngine::new(1);
    engine.set_hash_size(256);
    let mut board = BoardState::default();
    let mut prev: Option<(i32, String)> = None;
    println!("game {} engine={} plies={}  (loss = SF eval drop in engine's favour, centipawns)", game_no, a[3], moves.len());
    let mut drops = vec![];
    for i in 0..moves.len() {
        let engine_moves = (i % 2 == 0) == engine_white;
        let cur = match prev.take() { Some(p) => p, None => analyse(i) };
        if engine_moves && i >= 12 {
            let before = if engine_white { cur.0 } else { -cur.0 };
            let after_pair = analyse(i + 1);
            let after = if engine_white { after_pair.0 } else { -after_pair.0 };
            let loss = before - after;
            let pieces = board.all_pieces.count_ones();
            if let Ok(t) = std::env::var("TERMS") { if t.split(',').any(|x| x.parse::<usize>().ok() == Some(i + 1)) { println!("TERMS ply {} SF(engine pov)={:+} FEN {}", i + 1, before, board.to_fen()); Evaluator::debug_terms(&board); } }
            println!("traj ply {:>3} pieces={:>2} SFeval(engine pov)={:+5} engine_static={:+5}", i + 1, pieces, before, if engine_white { Evaluator::evaluate(&board) } else { -Evaluator::evaluate(&board) });
            if loss >= thr && before > -600 {
                // What does the engine think of this position, and what does it play now?
                let stat = Evaluator::evaluate(&board);
                engine.new_game();
                let r = engine.search(board.clone(), 64, Some(eng_ms));
                let again = r.best_move.map(|m| m.to_uci()).unwrap_or_default();
                println!("ply {:>3} (move {:>2}) pieces={:>2} played={} SFbest={} SFbefore={:+} SFafter={:+} LOSS={} | engine static={:+} search={:+} re-search plays {} {}",
                    i + 1, i / 2 + 1, pieces, moves[i], cur.1, before, after, loss, stat, r.score, again,
                    if again == moves[i] { "(SAME MOVE)" } else { "" });
                drops.push((i + 1, pieces, loss));
            }
            prev = Some(after_pair);
        }
        board.make_move_uci(&moves[i]).unwrap();
    }
    println!("blunders>={}cp: {}", thr, drops.len());
    for (p, pc, l) in &drops { println!("   ply {} pieces {} loss {}", p, pc, l); }
    send("quit");
}
