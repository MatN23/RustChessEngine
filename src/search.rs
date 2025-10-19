use crate::board::{BoardState, PIECE_VALUES};
use crate::movegen::{Move, MoveGenerator};
use crate::eval::Evaluator;
use crate::opening_book;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;

pub const INFINITY: i32 = 999999;
pub const MATE_SCORE: i32 = 900000;

pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub pv_lines: Vec<(Move, i32)>,
}

pub struct SearchEngine {
    tt: Arc<Mutex<TranspositionTable>>,
    threads: usize,
    nodes: Arc<Mutex<u64>>,
    stop: Arc<Mutex<bool>>,
    position_history: Arc<Mutex<HashMap<u64, u32>>>,
    killer_moves: Arc<Mutex<[[Option<Move>; 2]; 128]>>,
    history_table: Arc<Mutex<[[i32; 64]; 64]>>,
    countermove_table: Arc<Mutex<[[Option<Move>; 64]; 64]>>,
    multi_pv: usize,
}

impl SearchEngine {
    pub fn new(threads: usize) -> Self {
        SearchEngine {
            tt: Arc::new(Mutex::new(TranspositionTable::new(256))),
            threads,
            nodes: Arc::new(Mutex::new(0)),
            stop: Arc::new(Mutex::new(false)),
            position_history: Arc::new(Mutex::new(HashMap::new())),
            killer_moves: Arc::new(Mutex::new([[None; 2]; 128])),
            history_table: Arc::new(Mutex::new([[0; 64]; 64])),
            countermove_table: Arc::new(Mutex::new([[None; 64]; 64])),
            multi_pv: 1,
        }
    }

    pub fn search(
        &mut self,
        board: BoardState,
        max_depth: u8,
        time_ms: Option<u64>,
    ) -> SearchResult {
        *self.nodes.lock() = 0;
        *self.stop.lock() = false;

        *self.killer_moves.lock() = [[None; 2]; 128];
        *self.history_table.lock() = [[0; 64]; 64];
        *self.countermove_table.lock() = [[None; 64]; 64];

        // Check opening book
        if board.fullmove_number <= 15 {
            if let Some(book_move_uci) = opening_book::probe_book(&board.to_fen()) {
                if let Ok(mut temp_board) = BoardState::from_fen(&board.to_fen()) {
                    if temp_board.make_move_uci(&book_move_uci).is_ok() {
                        let moves = MoveGenerator::generate_legal_moves(&board);
                        for mv in moves {
                            if mv.to_uci() == book_move_uci {
                                println!("info string Opening book hit");
                                return SearchResult {
                                    best_move: Some(mv),
                                    score: 0,
                                    nodes: 0,
                                    pv_lines: vec![(mv, 0)],
                                };
                            }
                        }
                    }
                }
            }
        }

        let start_time = Instant::now();
        let time_limit = time_ms.map(|ms| Duration::from_millis(ms));

        let mut best_move = None;
        let mut best_score = 0;
        let mut prev_score = 0;
        let mut pv_lines = Vec::new();
        
        // 🆕 FIX #1: Track previous best move for stability
        let mut prev_best_move = None;

        // Iterative deepening
        for depth in 1..=max_depth {
            if *self.stop.lock() {
                break;
            }

            let (score, mv) = if depth >= 5 {
                self.search_aspiration(&board, depth, prev_score)
            } else {
                self.search_root(&board, depth, -INFINITY, INFINITY)
            };

            if *self.stop.lock() {
                break;
            }

            if let Some(m) = mv {
                // 🆕 FIX #2: PV Stability Check
                // Don't accept a new move if the score drops by more than 300cp
                // unless we're at very shallow depth or have no previous move
                let score_drop = prev_score - score;
                let should_reject = depth > 6 
                    && prev_best_move.is_some() 
                    && score_drop > 300 
                    && prev_score > -500; // Don't apply if already losing badly
                
                if should_reject {
                    println!("info string Score dropped {}cp, keeping previous move", score_drop);
                    // Keep the old move and break out
                    break;
                }
                
                best_move = Some(m);
                best_score = score;
                prev_score = score;
                prev_best_move = Some(m);
                pv_lines.push((m, score));

                let elapsed_ms = start_time.elapsed().as_millis();
                let nodes = *self.nodes.lock();
                let nps = if elapsed_ms > 0 {
                    (nodes as u128 * 1000 / elapsed_ms) as u64
                } else {
                    0
                };

                if score.abs() > MATE_SCORE - 100 {
                    let mate_in = (MATE_SCORE - score.abs() + 1) / 2;
                    println!(
                        "info depth {} score mate {} nodes {} nps {} time {} pv {}",
                        depth,
                        if score > 0 { mate_in } else { -mate_in },
                        nodes,
                        nps,
                        elapsed_ms,
                        m.to_uci()
                    );
                } else {
                    println!(
                        "info depth {} score cp {} nodes {} nps {} time {} pv {}",
                        depth, score, nodes, nps, elapsed_ms,
                        m.to_uci()
                    );
                }
            }

            // 🆕 FIX #3: Better time management
            // If we're losing badly and score is getting worse, use less time
            if let Some(limit) = time_limit {
                // Calculate how much time to use based on position
                let time_multiplier = if score < -500 {
                    // Losing badly - move fast
                    0.2
                } else if score < -200 {
                    // Slightly losing - moderate time
                    0.4
                } else if score > 300 {
                    // Winning - can move faster
                    0.4
                } else {
                    // Critical/equal position - use more time
                    0.6
            };
    
    let time_used_ratio = start_time.elapsed().as_millis() as f64 / limit.as_millis() as f64;
    
    if time_used_ratio > time_multiplier {
        break;
    }
}
        }

        SearchResult {
            best_move,
            score: best_score,
            nodes: *self.nodes.lock(),
            pv_lines,
        }
    }

    fn search_aspiration(&self, board: &BoardState, depth: u8, prev_score: i32) -> (i32, Option<Move>) {
        // 🆕 FIX #4: Wider initial aspiration window for stability
        let mut window = 50; // Increased from 30
        let mut alpha = prev_score - window;
        let mut beta = prev_score + window;

        loop {
            let (score, mv) = self.search_root(board, depth, alpha, beta);

            if *self.stop.lock() {
                return (prev_score, mv);
            }

            if score <= alpha {
                alpha = (alpha - window * 2).max(-INFINITY);
                beta = (alpha + beta) / 2;
            } else if score >= beta {
                beta = (beta + window * 2).min(INFINITY);
            } else {
                return (score, mv);
            }

            window = (window * 2).min(800); // Increased max window
        }
    }

    fn search_root(&self, board: &BoardState, depth: u8, mut alpha: i32, beta: i32) -> (i32, Option<Move>) {
        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if board.is_in_check(board.side_to_move) {
                (-MATE_SCORE, None)
            } else {
                (0, None)
            };
        }

        if moves.len() == 1 {
            return (0, Some(moves[0]));
        }

        self.order_moves(board, &mut moves, None, 0);

        let mut best_move = None;
        let mut best_score = -INFINITY;

        for mv in moves {
            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = if self.is_repetition(new_board.hash) {
                -25
            } else {
                -self.negamax(&new_board, depth - 1, -beta, -alpha, 1, true)
            };

            if score > best_score {
                best_score = score;
                best_move = Some(mv);
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta {
                break;
            }
        }

        (best_score, best_move)
    }

    fn negamax(&self, board: &BoardState, depth: u8, mut alpha: i32, beta: i32, ply: u8, pv_node: bool) -> i32 {
        if *self.nodes.lock() & 4095 == 0 {
            if *self.stop.lock() {
                return 0;
            }
        }

        *self.nodes.lock() += 1;

        if *self.stop.lock() {
            return 0;
        }

        if self.is_repetition(board.hash) {
            return if ply % 2 == 0 { -25 } else { 25 };
        }

        if board.halfmove_clock >= 100 {
            return 25;
        }

        // Mate distance pruning
        alpha = alpha.max(-MATE_SCORE + ply as i32);
        let beta_new = beta.min(MATE_SCORE - ply as i32 - 1);
        if alpha >= beta_new {
            return alpha;
        }

        let in_check = board.is_in_check(board.side_to_move);
        let mut depth = depth;

        // Check extension
        if in_check {
            depth += 1;
        }

        if depth == 0 {
            return self.quiescence(board, alpha, beta_new, 0);
        }

        // Probe transposition table
        let tt_entry = self.tt.lock().probe(board.hash);
        let mut tt_move = tt_entry.as_ref().and_then(|e| e.best_move);

        if let Some(entry) = &tt_entry {
            if entry.depth >= depth && !pv_node {
                match entry.flag {
                    TT_EXACT => return entry.score,
                    TT_ALPHA if entry.score <= alpha => return alpha,
                    TT_BETA if entry.score >= beta_new => return beta_new,
                    _ => {}
                }
            }
        }

        let static_eval = Evaluator::evaluate(board);

        // REVERSE FUTILITY PRUNING
        if !pv_node && !in_check && depth <= 7 {
            let rfp_margin = 85 * depth as i32;
            if static_eval - rfp_margin >= beta_new {
                return static_eval - rfp_margin;
            }
        }

        // NULL MOVE PRUNING with verification
        if !pv_node && !in_check && depth >= 3 && board.halfmove_clock < 90 {
            let mut null_board = board.clone();
            null_board.side_to_move = null_board.side_to_move.flip();
            null_board.ep_square = None;
            null_board.hash ^= crate::zobrist::ZOBRIST.side_key;
            
            let r = 3 + (depth / 4);
            let score = -self.negamax(&null_board, depth.saturating_sub(r), -beta_new, -beta_new + 1, ply + 1, false);
            
            if score >= beta_new {
                if depth < 12 {
                    return if score > MATE_SCORE - 100 { beta_new } else { score };
                }
                let verify = self.negamax(board, depth.saturating_sub(r), beta_new - 1, beta_new, ply, false);
                if verify >= beta_new {
                    return if score > MATE_SCORE - 100 { beta_new } else { score };
                }
            }
        }

        // Razoring
        if depth <= 3 && !in_check && !pv_node {
            let razor_margin = 350 + 180 * depth as i32;
            if static_eval + razor_margin < alpha {
                let q_score = self.quiescence(board, alpha, beta_new, 0);
                if q_score < alpha {
                    return q_score;
                }
            }
        }

        // INTERNAL ITERATIVE DEEPENING
        if tt_move.is_none() && depth >= 6 && pv_node {
            let iid_depth = depth - 2;
            self.negamax(board, iid_depth, alpha, beta_new, ply, true);
            tt_move = self.tt.lock().probe(board.hash).and_then(|e| e.best_move);
        }

        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if in_check {
                -MATE_SCORE + ply as i32
            } else {
                25
            };
        }

        self.order_moves(board, &mut moves, tt_move, ply);

        // SINGULAR EXTENSION
        if !in_check && depth >= 8 && tt_move.is_some() && pv_node {
            let tt_mv = tt_move.unwrap();
            if let Some(entry) = &tt_entry {
                if entry.depth >= depth - 3 && entry.flag == TT_BETA {
                    let s_beta = entry.score - 2 * depth as i32;
                    let mut excluded_score = -INFINITY;
                    
                    for mv in &moves {
                        if mv.from != tt_mv.from || mv.to != tt_mv.to {
                            let mut new_board = board.clone();
                            new_board.make_move(mv);
                            let score = -self.negamax(&new_board, (depth / 2).saturating_sub(1), -s_beta, -s_beta + 1, ply + 1, false);
                            excluded_score = excluded_score.max(score);
                            if excluded_score >= s_beta {
                                break;
                            }
                        }
                    }
                    
                    if excluded_score < s_beta {
                        depth += 1;
                    }
                }
            }
        }

        let mut best_score = -INFINITY;
        let mut best_move = None;
        let mut move_count = 0;
        let alpha_orig = alpha;
        let mut quiets_tried: Vec<Move> = Vec::new();

        for mv in moves {
            let mut new_board = board.clone();
            new_board.make_move(&mv);

            self.add_position(new_board.hash);

            // FUTILITY PRUNING
            let futile = !in_check && 
                        !new_board.is_in_check(new_board.side_to_move) &&
                        !mv.is_capture() && 
                        !mv.is_promotion() &&
                        move_count > 0 &&
                        depth <= 6;
            
            if futile {
                let futility_margin = 150 + 120 * depth as i32;
                if static_eval + futility_margin <= alpha {
                    self.remove_position(new_board.hash);
                    move_count += 1;
                    continue;
                }
            }

            let score = if move_count == 0 {
                -self.negamax(&new_board, depth - 1, -beta_new, -alpha, ply + 1, pv_node)
            } else {
                // IMPROVED LMR
                let reduction = if move_count >= 3 && depth >= 3 && !in_check && 
                                 !new_board.is_in_check(new_board.side_to_move) &&
                                 !mv.is_capture() && !mv.is_promotion() {
                    let base = ((depth as f32).ln() * (move_count as f32).ln() / 2.0) as u8;
                    let mut r = base.min(depth - 1).max(1);
                    
                    let killers = self.killer_moves.lock()[ply as usize];
                    let is_killer = killers.iter().any(|k| {
                        k.map_or(false, |killer| killer.from == mv.from && killer.to == mv.to)
                    });
                    
                    if is_killer {
                        r = r.saturating_sub(1);
                    }
                    
                    let history = self.history_table.lock()[mv.from as usize][mv.to as usize];
                    if history > 5000 {
                        r = r.saturating_sub(1);
                    }
                    
                    if !pv_node {
                        r += 1;
                    }
                    
                    r
                } else {
                    0
                };

                let mut score = -self.negamax(&new_board, depth.saturating_sub(reduction + 1), -alpha - 1, -alpha, ply + 1, false);
                
                if reduction > 0 && score > alpha {
                    score = -self.negamax(&new_board, depth - 1, -alpha - 1, -alpha, ply + 1, false);
                }

                if score > alpha && score < beta_new && pv_node {
                    score = -self.negamax(&new_board, depth - 1, -beta_new, -alpha, ply + 1, true);
                }

                score
            };

            self.remove_position(new_board.hash);
            move_count += 1;

            if score > best_score {
                best_score = score;
                best_move = Some(mv);
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta_new {
                if !mv.is_capture() {
                    self.update_killers(mv, ply);
                    self.update_history(mv, depth);
                    
                    for quiet in &quiets_tried {
                        if quiet.from != mv.from || quiet.to != mv.to {
                            let penalty = -(depth as i32) * (depth as i32);
                            self.update_history_raw(*quiet, penalty);
                        }
                    }
                }
                
                self.tt.lock().store(board.hash, depth, beta_new, TT_BETA, Some(mv));
                return beta_new;
            }
            
            if !mv.is_capture() && !mv.is_promotion() {
                quiets_tried.push(mv);
            }
        }

        let flag = if best_score <= alpha_orig {
            TT_ALPHA
        } else {
            TT_EXACT
        };
        self.tt.lock().store(board.hash, depth, best_score, flag, best_move);

        best_score
    }

    fn quiescence(&self, board: &BoardState, mut alpha: i32, beta: i32, depth: i8) -> i32 {
        *self.nodes.lock() += 1;

        if depth < -10 {
            return Evaluator::evaluate(board);
        }

        let stand_pat = Evaluator::evaluate(board);

        if stand_pat >= beta {
            return beta;
        }

        if stand_pat + 950 < alpha {
            return alpha;
        }

        if stand_pat > alpha {
            alpha = stand_pat;
        }

        let mut captures = MoveGenerator::generate_captures(board);

        if captures.is_empty() {
            return stand_pat;
        }

        self.order_captures(board, &mut captures);

        for mv in captures {
            if depth < -4 && !self.see_capture(board, &mv, 0) {
                continue;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = -self.quiescence(&new_board, -beta, -alpha, depth - 1);

            if score >= beta {
                return beta;
            }

            if score > alpha {
                alpha = score;
            }
        }

        alpha
    }

    fn order_moves(&self, board: &BoardState, moves: &mut Vec<Move>, tt_move: Option<Move>, ply: u8) {
        let killers = self.killer_moves.lock()[ply as usize];
        let history = self.history_table.lock();

        moves.sort_by_cached_key(|mv| {
            -self.score_move(board, mv, tt_move, &killers, &history)
        });
    }

    fn score_move(&self, board: &BoardState, mv: &Move, tt_move: Option<Move>, killers: &[Option<Move>; 2], history: &[[i32; 64]; 64]) -> i32 {
        if let Some(hash_mv) = tt_move {
            if mv.from == hash_mv.from && mv.to == hash_mv.to {
                return 10_000_000;
            }
        }

        if mv.is_capture() {
            return 9_000_000 + self.mvv_lva_score(board, mv);
        }

        if mv.is_promotion() {
            return 8_000_000;
        }

        if let Some(killer1) = killers[0] {
            if mv.from == killer1.from && mv.to == killer1.to {
                return 7_000_000;
            }
        }
        if let Some(killer2) = killers[1] {
            if mv.from == killer2.from && mv.to == killer2.to {
                return 6_900_000;
            }
        }

        history[mv.from as usize][mv.to as usize]
    }

    fn mvv_lva_score(&self, board: &BoardState, mv: &Move) -> i32 {
        let victim = if let Some((piece, _)) = board.piece_at(mv.to) {
            PIECE_VALUES[piece as usize]
        } else {
            100
        };

        let attacker = if let Some((piece, _)) = board.piece_at(mv.from) {
            PIECE_VALUES[piece as usize]
        } else {
            0
        };

        victim * 10 - attacker
    }

    fn order_captures(&self, board: &BoardState, captures: &mut Vec<Move>) {
        captures.sort_by_cached_key(|mv| {
            -self.mvv_lva_score(board, mv)
        });
    }

    fn see_capture(&self, board: &BoardState, mv: &Move, threshold: i32) -> bool {
        if !mv.is_capture() {
            return true;
        }

        let victim_value = if let Some((piece, _)) = board.piece_at(mv.to) {
            PIECE_VALUES[piece as usize]
        } else {
            100
        };

        let attacker_value = if let Some((piece, _)) = board.piece_at(mv.from) {
            PIECE_VALUES[piece as usize]
        } else {
            0
        };

        victim_value - attacker_value >= threshold
    }

    fn update_killers(&self, mv: Move, ply: u8) {
        let mut killers = self.killer_moves.lock();
        let ply_killers = &mut killers[ply as usize];
        
        if let Some(k1) = ply_killers[0] {
            if k1.from == mv.from && k1.to == mv.to {
                return;
            }
        }

        ply_killers[1] = ply_killers[0];
        ply_killers[0] = Some(mv);
    }

    fn update_history(&self, mv: Move, depth: u8) {
        let bonus = (depth as i32) * (depth as i32);
        self.update_history_raw(mv, bonus);
    }
    
    fn update_history_raw(&self, mv: Move, delta: i32) {
        let mut history = self.history_table.lock();
        history[mv.from as usize][mv.to as usize] += delta;
        
        if history[mv.from as usize][mv.to as usize].abs() > 10000 {
            for from in 0..64 {
                for to in 0..64 {
                    history[from][to] /= 2;
                }
            }
        }
    }

    fn is_repetition(&self, hash: u64) -> bool {
        self.position_history.lock().get(&hash).map_or(false, |&count| count >= 2)
    }

    fn add_position(&self, hash: u64) {
        *self.position_history.lock().entry(hash).or_insert(0) += 1;
    }

    fn remove_position(&self, hash: u64) {
        if let Some(count) = self.position_history.lock().get_mut(&hash) {
            *count -= 1;
        }
    }

    pub fn new_game(&mut self) {
        self.tt.lock().clear();
        *self.nodes.lock() = 0;
        self.position_history.lock().clear();
        *self.killer_moves.lock() = [[None; 2]; 128];
        *self.history_table.lock() = [[0; 64]; 64];
        *self.countermove_table.lock() = [[None; 64]; 64];
    }

    pub fn set_threads(&mut self, threads: usize) {
        self.threads = threads.clamp(1, 8);
    }
    
    pub fn set_multi_pv(&mut self, count: usize) {
        self.multi_pv = count.clamp(1, 5);
    }

    pub fn stop(&mut self) {
        *self.stop.lock() = true;
    }

    pub fn clear_tt(&mut self) {
        self.tt.lock().clear();
    }

    pub fn set_hash_size(&mut self, size_mb: usize) {
        self.tt.lock().resize(size_mb);
    }
}

const TT_EXACT: u8 = 0;
const TT_ALPHA: u8 = 1;
const TT_BETA: u8 = 2;

#[derive(Clone)]
struct TTEntry {
    hash: u64,
    depth: u8,
    score: i32,
    flag: u8,
    best_move: Option<Move>,
}

pub struct TranspositionTable {
    table: Vec<Option<TTEntry>>,
    size: usize,
}

impl TranspositionTable {
    fn new(size_mb: usize) -> Self {
        let size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        TranspositionTable {
            table: vec![None; size],
            size,
        }
    }

    fn probe(&self, hash: u64) -> Option<TTEntry> {
        let index = (hash as usize) % self.size;
        if let Some(entry) = &self.table[index] {
            if entry.hash == hash {
                return Some(entry.clone());
            }
        }
        None
    }

    fn store(&mut self, hash: u64, depth: u8, score: i32, flag: u8, best_move: Option<Move>) {
        let index = (hash as usize) % self.size;
        
        if let Some(entry) = &self.table[index] {
            if entry.hash == hash && entry.depth > depth {
                return;
            }
        }

        self.table[index] = Some(TTEntry {
            hash,
            depth,
            score,
            flag,
            best_move,
        });
    }

    fn clear(&mut self) {
        self.table = vec![None; self.size];
    }

    fn resize(&mut self, size_mb: usize) {
        self.size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        self.clear();
    }
}