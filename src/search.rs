use crate::board::{BoardState, Color, PIECE_VALUES};
use crate::movegen::{Move, MoveGenerator};
use crate::eval::Evaluator;
use crate::opening_book;
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use rayon::prelude::*;

pub const INFINITY: i32 = 999999;
pub const MATE_SCORE: i32 = 900000;
const MAX_PLY: usize = 128;
const MAX_THREADS: usize = 256;

// LMR reduction table
lazy_static::lazy_static! {
    static ref LMR_TABLE: [[u8; 64]; 64] = {
        let mut table = [[0u8; 64]; 64];
        for depth in 1..64 {
            for moves in 1..64 {
                let d = depth as f64;
                let m = moves as f64;
                table[depth][moves] = ((d.ln() * m.ln() / 2.0) as u8).min(depth as u8 - 1);
            }
        }
        table
    };
}

pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub pv_lines: Vec<(Move, i32)>,
}

pub struct SearchEngine {
    tt: Arc<RwLock<TranspositionTable>>,
    threads: usize,
    nodes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    multi_pv: usize,
    
    // Per-thread data
    thread_data: Arc<Vec<Mutex<ThreadData>>>,
}

struct ThreadData {
    killer_moves: [[Option<Move>; 2]; MAX_PLY],
    history_table: [[i32; 64]; 64],
    countermove_table: [[Option<Move>; 64]; 64],
    nodes_searched: u64,
    pv_table: [[Option<Move>; MAX_PLY]; MAX_PLY],
    pv_length: [usize; MAX_PLY],
}

impl ThreadData {
    fn new() -> Self {
        ThreadData {
            killer_moves: [[None; 2]; MAX_PLY],
            history_table: [[0; 64]; 64],
            countermove_table: [[None; 64]; 64],
            nodes_searched: 0,
            pv_table: [[None; MAX_PLY]; MAX_PLY],
            pv_length: [0; MAX_PLY],
        }
    }

    fn clear(&mut self) {
        self.killer_moves = [[None; 2]; MAX_PLY];
        self.history_table = [[0; 64]; 64];
        self.countermove_table = [[None; 64]; 64];
        self.nodes_searched = 0;
        self.pv_table = [[None; MAX_PLY]; MAX_PLY];
        self.pv_length = [0; MAX_PLY];
    }
}

impl SearchEngine {
    pub fn new(threads: usize) -> Self {
        let threads = threads.clamp(1, MAX_THREADS);
        let mut thread_data = Vec::new();
        for _ in 0..threads {
            thread_data.push(Mutex::new(ThreadData::new()));
        }

        SearchEngine {
            tt: Arc::new(RwLock::new(TranspositionTable::new(512))),
            threads,
            nodes: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            multi_pv: 1,
            thread_data: Arc::new(thread_data),
        }
    }

    pub fn search(
        &mut self,
        board: BoardState,
        max_depth: u8,
        time_ms: Option<u64>,
    ) -> SearchResult {
        self.nodes.store(0, Ordering::Relaxed);
        self.stop.store(false, Ordering::Relaxed);

        // Clear thread data
        for thread_data in self.thread_data.iter() {
            thread_data.lock().clear();
        }

        // Opening book probe
        if board.fullmove_number <= 15 {
            if let Some(book_move_uci) = opening_book::probe_book(&board.to_fen()) {
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

        let start_time = Instant::now();
        let time_limit = time_ms.map(Duration::from_millis);

        let mut best_move = None;
        let mut best_score = 0;
        let mut prev_score = 0;
        let pv_lines = Vec::new();

        // Iterative deepening
        for depth in 1..=max_depth {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            let soft_limit = time_limit.map(|t| t.mul_f64(0.4));
            let hard_limit = time_limit;

            let (score, mv, pv) = if depth >= 5 {
                self.search_aspiration(&board, depth, prev_score, start_time, soft_limit, hard_limit)
            } else {
                self.search_root(&board, depth, -INFINITY, INFINITY, start_time, soft_limit, hard_limit)
            };

            if self.stop.load(Ordering::Relaxed) && depth > 1 {
                break;
            }

            if let Some(m) = mv {
                let score_drop = prev_score - score;
                
                // PV stability check
                let should_reject = depth > 7
                    && best_move.is_some()
                    && score_drop > 250
                    && prev_score > -400
                    && prev_score < MATE_SCORE - 1000;

                if should_reject {
                    println!("info string Score drop {}cp, keeping previous move", score_drop);
                    break;
                }

                best_move = Some(m);
                best_score = score;
                prev_score = score;

                let elapsed_ms = start_time.elapsed().as_millis();
                let nodes = self.nodes.load(Ordering::Relaxed);
                let nps = if elapsed_ms > 0 {
                    (nodes as u128 * 1000 / elapsed_ms) as u64
                } else {
                    0
                };

                let mut pv_str = String::new();
                for pv_move in pv.iter().take(10) {
                    pv_str.push_str(&format!("{} ", pv_move.to_uci()));
                }

                if score.abs() > MATE_SCORE - 100 {
                    let mate_in = (MATE_SCORE - score.abs() + 1) / 2;
                    println!(
                        "info depth {} score mate {} nodes {} nps {} time {} pv {}",
                        depth,
                        if score > 0 { mate_in } else { -mate_in },
                        nodes,
                        nps,
                        elapsed_ms,
                        pv_str.trim()
                    );
                } else {
                    println!(
                        "info depth {} score cp {} nodes {} nps {} time {} pv {}",
                        depth, score, nodes, nps, elapsed_ms, pv_str.trim()
                    );
                }

                // Smart time management
                if let Some(soft) = soft_limit {
                    if start_time.elapsed() > soft {
                        let time_ratio = start_time.elapsed().as_millis() as f64 / soft.as_millis() as f64;
                        
                        if score < -500 || (time_ratio > 1.5 && score_drop.abs() < 30) {
                            break;
                        }
                    }
                }
            }

            if let Some(hard) = hard_limit {
                if start_time.elapsed() > hard.mul_f64(0.9) {
                    break;
                }
            }
        }

        SearchResult {
            best_move,
            score: best_score,
            nodes: self.nodes.load(Ordering::Relaxed),
            pv_lines,
        }
    }

    fn search_aspiration(
        &self,
        board: &BoardState,
        depth: u8,
        prev_score: i32,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, Vec<Move>) {
        let mut window = 50;
        let mut alpha = prev_score - window;
        let mut beta = prev_score + window;
        let mut fail_high_count = 0;
        let mut fail_low_count = 0;

        loop {
            let (score, mv, pv) = self.search_root(board, depth, alpha, beta, start_time, soft_limit, hard_limit);

            if self.stop.load(Ordering::Relaxed) {
                return (prev_score, mv, pv);
            }

            if score <= alpha {
                // Fail low
                fail_low_count += 1;
                beta = (alpha + beta) / 2;
                alpha = (alpha - window * (1 + fail_low_count)).max(-INFINITY);
                println!("info string Fail low, widening window to [{}, {}]", alpha, beta);
            } else if score >= beta {
                // Fail high
                fail_high_count += 1;
                beta = (beta + window * (1 + fail_high_count)).min(INFINITY);
                println!("info string Fail high, widening window to [{}, {}]", alpha, beta);
            } else {
                return (score, mv, pv);
            }

            window = (window * 2).min(1000);

            // Emergency exit on extreme fails
            if fail_high_count + fail_low_count > 5 {
                return self.search_root(board, depth, -INFINITY, INFINITY, start_time, soft_limit, hard_limit);
            }
        }
    }

    fn search_root(
        &self,
        board: &BoardState,
        depth: u8,
        alpha: i32,
        beta: i32,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, Vec<Move>) {
        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if board.is_in_check(board.side_to_move) {
                (-MATE_SCORE, None, vec![])
            } else {
                (0, None, vec![])
            };
        }

        if moves.len() == 1 {
            return (0, Some(moves[0]), vec![moves[0]]);
        }

        // Order moves using main thread data
        let mut thread_data = self.thread_data[0].lock();
        self.order_moves_internal(board, &mut moves, None, 0, &mut thread_data);
        drop(thread_data);

        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_pv = Vec::new();

        // Lazy SMP: Launch parallel search on multiple threads
        if self.threads > 1 && depth >= 6 {
            let results: Vec<_> = (0..self.threads)
                .into_par_iter()
                .map(|thread_id| {
                    if self.stop.load(Ordering::Relaxed) {
                        return (-INFINITY, None, vec![]);
                    }

                    let depth_variation = if thread_id > 0 {
                        // Vary depth for helper threads
                        let offset = (thread_id as i32) % 4 - 1;
                        (depth as i32 + offset).max(1).min(depth as i32) as u8
                    } else {
                        depth
                    };

                    self.search_root_thread(
                        board,
                        depth_variation,
                        alpha,
                        beta,
                        thread_id,
                        start_time,
                        soft_limit,
                        hard_limit,
                    )
                })
                .collect();

            // Select best result
            for (score, mv, pv) in results {
                if score > best_score {
                    best_score = score;
                    best_move = mv;
                    best_pv = pv;
                }
            }
        } else {
            // Single-threaded search
            let (score, mv, pv) = self.search_root_thread(
                board,
                depth,
                alpha,
                beta,
                0,
                start_time,
                soft_limit,
                hard_limit,
            );
            best_score = score;
            best_move = mv;
            best_pv = pv;
        }

        (best_score, best_move, best_pv)
    }

    fn search_root_thread(
        &self,
        board: &BoardState,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        thread_id: usize,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
    ) -> (i32, Option<Move>, Vec<Move>) {
        let mut moves = MoveGenerator::generate_legal_moves(board);
        let mut thread_data = self.thread_data[thread_id].lock();
        self.order_moves_internal(board, &mut moves, None, 0, &mut thread_data);
        
        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_pv = Vec::new();
        let mut move_count = 0;

        for mv in moves {
            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                break;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = if move_count == 0 {
                // Full window search for first move
                -self.pvs(&new_board, depth - 1, -beta, -alpha, 1, true, thread_id, start_time, soft_limit, hard_limit, &mut thread_data)
            } else {
                // PVS: null window search
                let mut score = -self.pvs(&new_board, depth - 1, -alpha - 1, -alpha, 1, false, thread_id, start_time, soft_limit, hard_limit, &mut thread_data);
                
                if score > alpha && score < beta {
                    // Re-search with full window
                    score = -self.pvs(&new_board, depth - 1, -beta, -alpha, 1, true, thread_id, start_time, soft_limit, hard_limit, &mut thread_data);
                }
                score
            };

            move_count += 1;

            if score > best_score {
                best_score = score;
                best_move = Some(mv);
                
                // Copy PV
                best_pv.clear();
                best_pv.push(mv);
                if thread_data.pv_length[1] > 0 {
                    for i in 0..thread_data.pv_length[1] {
                        if let Some(pv_move) = thread_data.pv_table[1][i] {
                            best_pv.push(pv_move);
                        }
                    }
                }
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta {
                self.update_killers_internal(mv, 0, &mut thread_data);
                self.update_history_internal(mv, depth, &mut thread_data);
                break;
            }
        }

        drop(thread_data);
        (best_score, best_move, best_pv)
    }

    #[allow(clippy::too_many_arguments)]
    fn pvs(
        &self,
        board: &BoardState,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        pv_node: bool,
        thread_id: usize,
        start_time: Instant,
        soft_limit: Option<Duration>,
        hard_limit: Option<Duration>,
        thread_data: &mut ThreadData,
    ) -> i32 {
        // Periodic stop check
        thread_data.nodes_searched += 1;
        if thread_data.nodes_searched & 2047 == 0 {
            self.nodes.fetch_add(2048, Ordering::Relaxed);
            thread_data.nodes_searched = 0;

            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                return 0;
            }
        }

        if self.stop.load(Ordering::Relaxed) {
            return 0;
        }

        // Draw detection
        if board.halfmove_clock >= 100 || board.is_repetition() {
            return 0;
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
            depth = depth.saturating_add(1);
        }

        // Quiescence at leaf nodes
        if depth == 0 {
            return self.quiescence(board, alpha, beta_new, 0, thread_data);
        }

        // TT probe
        let tt_entry = self.tt.read().probe(board.hash);
        let mut tt_move = tt_entry.as_ref().and_then(|e| e.best_move);

        if let Some(entry) = &tt_entry {
            if entry.depth >= depth && !pv_node && ply > 0 {
                match entry.flag {
                    TT_EXACT => return entry.score,
                    TT_ALPHA if entry.score <= alpha => return alpha,
                    TT_BETA if entry.score >= beta_new => return beta_new,
                    _ => {}
                }
            }
        }

        let static_eval = Evaluator::evaluate(board);

        // Reverse futility pruning
        if !pv_node && !in_check && depth <= 7 {
            let rfp_margin = 90 * depth as i32;
            if static_eval - rfp_margin >= beta_new {
                return static_eval - rfp_margin;
            }
        }

        // Null move pruning with verification
        if !pv_node && !in_check && depth >= 3 && board.halfmove_clock < 90 {
            let has_pieces = (board.pieces[board.side_to_move as usize][2] 
                | board.pieces[board.side_to_move as usize][3]
                | board.pieces[board.side_to_move as usize][4]
                | board.pieces[board.side_to_move as usize][5]) != 0;

            if has_pieces && static_eval >= beta_new {
                let mut null_board = board.clone();
                null_board.side_to_move = null_board.side_to_move.flip();
                null_board.ep_square = None;
                null_board.hash ^= crate::zobrist::ZOBRIST.side_key;

                let r = 3 + (depth / 4) + ((static_eval - beta_new) / 200).clamp(0, 2) as u8;
                let score = -self.pvs(&null_board, depth.saturating_sub(r), -beta_new, -beta_new + 1, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);

                if score >= beta_new {
                    if depth < 12 {
                        return if score > MATE_SCORE - 100 { beta_new } else { score };
                    }
                    // Verification search
                    let verify = self.pvs(board, depth.saturating_sub(r), beta_new - 1, beta_new, ply, false, thread_id, start_time, soft_limit, hard_limit, thread_data);
                    if verify >= beta_new {
                        return if score > MATE_SCORE - 100 { beta_new } else { score };
                    }
                }
            }
        }

        // Razoring
        if depth <= 3 && !in_check && !pv_node {
            let razor_margin = 350 + 200 * depth as i32;
            if static_eval + razor_margin < alpha {
                let q_score = self.quiescence(board, alpha, beta_new, 0, thread_data);
                if q_score < alpha {
                    return q_score.max(alpha - razor_margin);
                }
            }
        }

        // Internal iterative deepening
        if tt_move.is_none() && depth >= 6 && pv_node {
            let iid_depth = depth.saturating_sub(2);
            self.pvs(board, iid_depth, alpha, beta_new, ply, true, thread_id, start_time, soft_limit, hard_limit, thread_data);
            let entry = self.tt.read().probe(board.hash);
            tt_move = entry.and_then(|e| e.best_move);
        }

        let mut moves = MoveGenerator::generate_legal_moves(board);

        if moves.is_empty() {
            return if in_check {
                -MATE_SCORE + ply as i32
            } else {
                0
            };
        }

        self.order_moves_internal(board, &mut moves, tt_move, ply, thread_data);

        let mut best_score = -INFINITY;
        let mut best_move = None;
        let mut move_count = 0;
        let alpha_orig = alpha;
        let mut quiets_tried: Vec<Move> = Vec::new();

        thread_data.pv_length[ply] = 0;

        for mv in moves {
            if self.check_time_abort(start_time, soft_limit, hard_limit) {
                break;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            // Futility pruning
            let futile = !in_check
                && !new_board.is_in_check(new_board.side_to_move)
                && !mv.is_capture()
                && !mv.is_promotion()
                && move_count > 0
                && depth <= 6;

            if futile {
                let futility_margin = 150 + 130 * depth as i32;
                if static_eval + futility_margin <= alpha {
                    move_count += 1;
                    continue;
                }
            }

            let gives_check = new_board.is_in_check(new_board.side_to_move);
            let mut extension = 0;

            // Passed pawn extension
            if !gives_check && mv.from / 8 == 6 && board.side_to_move == Color::White && !mv.is_capture() {
                let pawn_bb = board.pieces[0][1];
                if (pawn_bb & (1u64 << mv.from)) != 0 {
                    extension = 1;
                }
            } else if !gives_check && mv.from / 8 == 1 && board.side_to_move == Color::Black && !mv.is_capture() {
                let pawn_bb = board.pieces[1][1];
                if (pawn_bb & (1u64 << mv.from)) != 0 {
                    extension = 1;
                }
            }

            let score = if move_count == 0 {
                // First move: full window PVS
                -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -beta_new, -alpha, ply + 1, pv_node, thread_id, start_time, soft_limit, hard_limit, thread_data)
            } else {
                // Late move reductions
                let reduction = if move_count >= 3 && depth >= 3 && !in_check && !gives_check && !mv.is_capture() && !mv.is_promotion() {
                    let base = LMR_TABLE[depth.min(63) as usize][move_count.min(63)];
                    let mut r = base;

                    // Reduce less in PV nodes
                    if pv_node {
                        r = r.saturating_sub(1);
                    }

                    // Reduce less for killer moves
                    let is_killer = thread_data.killer_moves[ply].iter().any(|k| {
                        k.map_or(false, |killer| killer.from == mv.from && killer.to == mv.to)
                    });
                    if is_killer {
                        r = r.saturating_sub(1);
                    }

                    // Reduce less for good history
                    let history = thread_data.history_table[mv.from as usize][mv.to as usize];
                    if history > 5000 {
                        r = r.saturating_sub(1);
                    } else if history < -5000 {
                        r = r.saturating_add(1);
                    }

                    r.min(depth.saturating_sub(1))
                } else {
                    0
                };

                // Null window search with reduction
                let mut score = -self.pvs(&new_board, depth.saturating_sub(reduction + 1).saturating_add(extension), -alpha - 1, -alpha, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);

                // Re-search if reduced and score beats alpha
                if reduction > 0 && score > alpha {
                    score = -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -alpha - 1, -alpha, ply + 1, false, thread_id, start_time, soft_limit, hard_limit, thread_data);
                }

                // Re-search with full window if score is in (alpha, beta)
                if score > alpha && score < beta_new && pv_node {
                    score = -self.pvs(&new_board, depth.saturating_sub(1).saturating_add(extension), -beta_new, -alpha, ply + 1, true, thread_id, start_time, soft_limit, hard_limit, thread_data);
                }

                score
            };

            move_count += 1;

            if score > best_score {
                best_score = score;
                best_move = Some(mv);

                // Update PV
                thread_data.pv_table[ply][0] = Some(mv);
                thread_data.pv_length[ply] = 1;
                if ply + 1 < MAX_PLY && thread_data.pv_length[ply + 1] > 0 {
                    for i in 0..thread_data.pv_length[ply + 1] {
                        thread_data.pv_table[ply][i + 1] = thread_data.pv_table[ply + 1][i];
                    }
                    thread_data.pv_length[ply] += thread_data.pv_length[ply + 1];
                }
            }

            if score > alpha {
                alpha = score;
            }

            if score >= beta_new {
                // Beta cutoff
                if !mv.is_capture() {
                    self.update_killers_internal(mv, ply, thread_data);
                    self.update_history_internal(mv, depth, thread_data);

                    // History penalty for quiet moves that didn't cause cutoff
                    for quiet in &quiets_tried {
                        if quiet.from != mv.from || quiet.to != mv.to {
                            let penalty = -(depth as i32) * (depth as i32);
                            self.update_history_raw_internal(*quiet, penalty, thread_data);
                        }
                    }
                }

                self.tt.write().store(board.hash, depth, beta_new, TT_BETA, Some(mv));
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

        self.tt.write().store(board.hash, depth, best_score, flag, best_move);
        best_score
    }

    fn quiescence(&self, board: &BoardState, mut alpha: i32, beta: i32, depth: i8, thread_data: &mut ThreadData) -> i32 {
        thread_data.nodes_searched += 1;

        if depth < -10 {
            return Evaluator::evaluate(board);
        }

        let stand_pat = Evaluator::evaluate(board);

        if stand_pat >= beta {
            return beta;
        }

        // Delta pruning
        let delta = 950;
        if stand_pat + delta < alpha {
            return alpha;
        }

        if stand_pat > alpha {
            alpha = stand_pat;
        }

        let mut captures = MoveGenerator::generate_captures(board);

        if captures.is_empty() {
            return stand_pat;
        }

        self.order_captures_internal(board, &mut captures);

        for mv in captures {
            // Delta pruning with SEE
            if depth < -4 && !self.see_capture(board, &mv, 0) {
                continue;
            }

            let mut new_board = board.clone();
            new_board.make_move(&mv);

            let score = -self.quiescence(&new_board, -beta, -alpha, depth - 1, thread_data);

            if score >= beta {
                return beta;
            }

            if score > alpha {
                alpha = score;
            }
        }

        alpha
    }

    fn order_moves_internal(&self, board: &BoardState, moves: &mut Vec<Move>, tt_move: Option<Move>, ply: usize, thread_data: &mut ThreadData) {
        let killers = thread_data.killer_moves[ply];
        let history = &thread_data.history_table;

        moves.sort_by_cached_key(|mv| {
            -self.score_move_internal(board, mv, tt_move, &killers, history)
        });
    }

    fn score_move_internal(&self, board: &BoardState, mv: &Move, tt_move: Option<Move>, killers: &[Option<Move>; 2], history: &[[i32; 64]; 64]) -> i32 {
        // TT move has highest priority
        if let Some(hash_mv) = tt_move {
            if mv.from == hash_mv.from && mv.to == hash_mv.to {
                return 10_000_000;
            }
        }

        // Winning captures (MVV-LVA)
        if mv.is_capture() {
            let mut score = 9_000_000 + self.mvv_lva_score(board, mv);
            // Bonus for captures that pass SEE
            if self.see_capture(board, mv, 0) {
                score += 100_000;
            }
            return score;
        }

        // Promotions
        if mv.is_promotion() {
            return 8_000_000 + match mv.promotion_piece() {
                Some(crate::board::Piece::Queen) => 4000,
                Some(crate::board::Piece::Knight) => 3000,
                Some(crate::board::Piece::Rook) => 2000,
                Some(crate::board::Piece::Bishop) => 1000,
                _ => 0,
            };
        }

        // First killer move
        if let Some(killer1) = killers[0] {
            if mv.from == killer1.from && mv.to == killer1.to {
                return 7_000_000;
            }
        }

        // Second killer move
        if let Some(killer2) = killers[1] {
            if mv.from == killer2.from && mv.to == killer2.to {
                return 6_900_000;
            }
        }

        // History heuristic
        history[mv.from as usize][mv.to as usize].clamp(-10_000, 10_000)
    }

    fn mvv_lva_score(&self, board: &BoardState, mv: &Move) -> i32 {
        let victim = if let Some((piece, _)) = board.piece_at(mv.to) {
            PIECE_VALUES[piece as usize]
        } else {
            100 // En passant
        };

        let attacker = if let Some((piece, _)) = board.piece_at(mv.from) {
            PIECE_VALUES[piece as usize]
        } else {
            0
        };

        // MVV-LVA: prioritize capturing valuable pieces with less valuable pieces
        victim * 10 - attacker / 10
    }

    fn order_captures_internal(&self, board: &BoardState, captures: &mut Vec<Move>) {
        captures.sort_by_cached_key(|mv| {
            let mut score = -self.mvv_lva_score(board, mv);
            // Prioritize captures that pass SEE
            if self.see_capture(board, mv, 0) {
                score -= 100_000;
            }
            score
        });
    }

    fn see_capture(&self, board: &BoardState, mv: &Move, threshold: i32) -> bool {
        if !mv.is_capture() {
            return true;
        }

        let victim_value = if let Some((piece, _)) = board.piece_at(mv.to) {
            PIECE_VALUES[piece as usize]
        } else {
            100 // En passant
        };

        let attacker_value = if let Some((piece, _)) = board.piece_at(mv.from) {
            PIECE_VALUES[piece as usize]
        } else {
            0
        };

        // Simple SEE approximation
        let gain = victim_value - attacker_value;
        gain >= threshold
    }

    fn update_killers_internal(&self, mv: Move, ply: usize, thread_data: &mut ThreadData) {
        if ply >= MAX_PLY {
            return;
        }

        let ply_killers = &mut thread_data.killer_moves[ply];

        // Check if already a killer
        if let Some(k1) = ply_killers[0] {
            if k1.from == mv.from && k1.to == mv.to {
                return;
            }
        }

        // Shift killers
        ply_killers[1] = ply_killers[0];
        ply_killers[0] = Some(mv);
    }

    fn update_history_internal(&self, mv: Move, depth: u8, thread_data: &mut ThreadData) {
        let bonus = (depth as i32) * (depth as i32);
        self.update_history_raw_internal(mv, bonus, thread_data);
    }

    fn update_history_raw_internal(&self, mv: Move, delta: i32, thread_data: &mut ThreadData) {
        let entry = &mut thread_data.history_table[mv.from as usize][mv.to as usize];
        *entry += delta;

        // Gravity: prevent values from growing too large
        if entry.abs() > 10000 {
            for from in 0..64 {
                for to in 0..64 {
                    thread_data.history_table[from][to] /= 2;
                }
            }
        }
    }

    fn check_time_abort(&self, start_time: Instant, _soft_limit: Option<Duration>, hard_limit: Option<Duration>) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }

        if let Some(hard) = hard_limit {
            if start_time.elapsed() > hard {
                self.stop.store(true, Ordering::Relaxed);
                return true;
            }
        }

        false
    }

    pub fn new_game(&mut self) {
        self.tt.write().clear();
        self.nodes.store(0, Ordering::Relaxed);
        
        for thread_data in self.thread_data.iter() {
            thread_data.lock().clear();
        }
    }

    pub fn set_threads(&mut self, threads: usize) {
        let new_threads = threads.clamp(1, MAX_THREADS);
        if new_threads == self.threads {
            return;
        }

        self.threads = new_threads;
        
        // Rebuild thread data
        let mut new_thread_data = Vec::new();
        for _ in 0..new_threads {
            new_thread_data.push(Mutex::new(ThreadData::new()));
        }
        self.thread_data = Arc::new(new_thread_data);
    }

    pub fn set_multi_pv(&mut self, count: usize) {
        self.multi_pv = count.clamp(1, 5);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn clear_tt(&mut self) {
        self.tt.write().clear();
    }

    pub fn set_hash_size(&mut self, size_mb: usize) {
        self.tt.write().resize(size_mb);
    }
}

// Transposition Table Entry Flags
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
    age: u8,
}

pub struct TranspositionTable {
    table: Vec<Option<TTEntry>>,
    size: usize,
    current_age: u8,
}

impl TranspositionTable {
    fn new(size_mb: usize) -> Self {
        let size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        TranspositionTable {
            table: vec![None; size],
            size,
            current_age: 0,
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

        // Replacement strategy
        let should_replace = if let Some(entry) = &self.table[index] {
            if entry.hash == hash {
                // Always replace if same position
                true
            } else {
                // Replace based on depth and age
                let depth_diff = depth as i32 - entry.depth as i32;
                let age_diff = self.current_age.wrapping_sub(entry.age);
                
                // Replace if:
                // 1. Much deeper search
                // 2. Old entry
                // 3. Depth is similar and entry is old
                depth_diff >= 3 || age_diff >= 4 || (depth_diff >= 0 && age_diff >= 2)
            }
        } else {
            true
        };

        if should_replace {
            self.table[index] = Some(TTEntry {
                hash,
                depth,
                score,
                flag,
                best_move,
                age: self.current_age,
            });
        }
    }

    fn clear(&mut self) {
        self.table = vec![None; self.size];
        self.current_age = 0;
    }

    fn resize(&mut self, size_mb: usize) {
        self.size = (size_mb * 1024 * 1024) / std::mem::size_of::<Option<TTEntry>>();
        self.table = vec![None; self.size];
        self.current_age = 0;
    }

    #[allow(dead_code)]
    fn increment_age(&mut self) {
        self.current_age = self.current_age.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BoardState;

    #[test]
    fn test_search_basic() {
        let board = BoardState::default();
        let mut engine = SearchEngine::new(1);
        let result = engine.search(board, 5, None);
        assert!(result.best_move.is_some());
    }

    #[test]
    fn test_search_parallel() {
        let board = BoardState::default();
        let mut engine = SearchEngine::new(4);
        let result = engine.search(board, 5, None);
        assert!(result.best_move.is_some());
        assert!(result.nodes > 0);
    }

    #[test]
    fn test_mate_in_one() {
        // Scholar's mate setup: 1 move to checkmate
        let fen = "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let board = BoardState::from_fen(fen).unwrap();
        let mut engine = SearchEngine::new(1);
        let result = engine.search(board, 10, None);
        
        // Should find a defensive move or recognize it's mate
        assert!(result.score < -MATE_SCORE + 100 || result.best_move.is_some());
    }

    #[test]
    fn test_time_management() {
        let board = BoardState::default();
        let mut engine = SearchEngine::new(1);
        
        let start = std::time::Instant::now();
        engine.search(board, 50, Some(1000));
        let elapsed = start.elapsed();
        
        // Should respect time limit (with some tolerance)
        assert!(elapsed.as_millis() < 1500);
    }

    #[test]
    fn test_transposition_table() {
        let mut tt = TranspositionTable::new(16);
        let test_move = Move::new(12, 20, 0);
        
        tt.store(12345, 5, 100, TT_EXACT, Some(test_move));
        
        let entry = tt.probe(12345);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().score, 100);
    }

    #[test]
    fn test_thread_scaling() {
        let board = BoardState::default();
        
        // Test with 1 thread
        let mut engine1 = SearchEngine::new(1);
        let start1 = std::time::Instant::now();
        engine1.search(board.clone(), 6, None);
        let time1 = start1.elapsed();
        
        // Test with 4 threads
        let mut engine4 = SearchEngine::new(4);
        let start4 = std::time::Instant::now();
        engine4.search(board, 6, None);
        let time4 = start4.elapsed();
        
        // 4 threads should be faster (though not 4x due to overhead)
        println!("1 thread: {:?}, 4 threads: {:?}", time1, time4);
        assert!(time4 < time1);
    }

    #[test]
    fn test_lmr_table() {
        // Verify LMR table is reasonable
        assert_eq!(LMR_TABLE[1][1], 0);
        assert!(LMR_TABLE[10][10] > 0);
        assert!(LMR_TABLE[20][30] < 20);
    }

    #[test]
    fn test_mvv_lva() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let board = BoardState::from_fen(fen).unwrap();
        let engine = SearchEngine::new(1);
        
        // Create two test moves
        let move1 = Move::new(12, 20, 4); // Pawn captures
        let move2 = Move::new(1, 18, 4);  // Knight captures
        
        // MVV-LVA should prefer lower-value attacker for same victim
        // This is a simple sanity check
        let score1 = engine.mvv_lva_score(&board, &move1);
        let score2 = engine.mvv_lva_score(&board, &move2);
        
        println!("Pawn capture: {}, Knight capture: {}", score1, score2);
    }

    #[test]
    fn test_killer_moves() {
        let mut thread_data = ThreadData::new();
        let test_move = Move::new(12, 20, 0);
        
        let engine = SearchEngine::new(1);
        engine.update_killers_internal(test_move, 0, &mut thread_data);
        
        assert_eq!(thread_data.killer_moves[0][0], Some(test_move));
    }

    #[test]
    fn test_history_table() {
        let mut thread_data = ThreadData::new();
        let test_move = Move::new(12, 20, 0);
        
        let engine = SearchEngine::new(1);
        engine.update_history_internal(test_move, 5, &mut thread_data);
        
        let score = thread_data.history_table[12][20];
        assert!(score > 0);
    }
}