//! Native Rust Lichess bot. Replaces `lichess_bot_rust.py` (no Python, no PyO3,
//! no UCI subprocess): the bot calls `SearchEngine` directly and keeps the real
//! `BoardState` (including repetition history) for the whole game.
//!
//! Usage:
//!   LICHESS_TOKEN=xxxx cargo run --release --no-default-features --bin lichess_bot
//!   cargo run --release --no-default-features --bin lichess_bot -- xxxx

#![allow(dead_code, unused_imports, unused_variables)]

mod bitboard;
mod board;
mod eval;
mod lichess;
mod movegen;
mod opening_book;
mod search;
mod zobrist;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match lichess::Config::from_env(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}\n");
            eprintln!("Usage: lichess_bot [LICHESS_TOKEN]   (or set LICHESS_TOKEN)");
            eprintln!("Optional env: LICHESS_URL ENGINE_THREADS ENGINE_HASH MOVE_OVERHEAD");
            eprintln!("              SEARCH_DEPTH PONDER(1/0) PONDER_MAX_MS MAX_GAMES");
            std::process::exit(2);
        }
    };

    let bot = match lichess::bot::Bot::connect(cfg) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    if let Err(e) = bot.run(&shutdown) {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
