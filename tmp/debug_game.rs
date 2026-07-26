use chess_engine::board::BoardState;
use chess_engine::search::SearchEngine;

fn main() {
    let fen = "rnbq1b1r/pp3kp1/5n1p/2pp4/8/1PpQ2P1/P1P1PPBP/R1B1K2R w KQ - 0 11";
    let mut board = BoardState::from_fen(fen).unwrap();
    let mut searcher = SearchEngine::new(1); // 1 thread
    
    let mv = searcher.get_best_move(&board, 5000); // 5 seconds
    println!("Best move: {}", mv);
    
    // Also, evaluate the static score!
    let eval = chess_engine::eval::Evaluator::evaluate(&board);
    println!("Static Eval: {}", eval);
}
