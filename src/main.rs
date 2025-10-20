mod board;
mod bitboard;
mod movegen;
mod search;
mod eval;
mod zobrist;
mod opening_book;
mod uci;

fn main() {
    let mut engine = uci::UCIEngine::new();
    engine.run();
}