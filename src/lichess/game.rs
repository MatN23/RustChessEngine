//! One Lichess game: stream handling, move selection, pondering.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::client::{ApiError, LichessClient, NdjsonStream, StreamItem};
use super::{timing, Config, START_FEN};
use crate::board::{BoardState, Color};
use crate::movegen::MoveGenerator;
use crate::search::SearchEngine;

/// Search recursion + per-ply `BoardState`/`MoveList` frames are large; Rust's
/// default 2 MiB spawned-thread stack is not a comfortable margin. Pages are
/// only committed when touched, so this costs nothing in practice.
pub const THREAD_STACK: usize = 64 * 1024 * 1024;

/// Test-only: parks the ponder thread between its last "am I still wanted?"
/// check and the call into `engine.search()` (which resets the stop flag), so a
/// test can deterministically hit the lost-stop window.
#[cfg(test)]
pub static PONDER_PRE_SEARCH_DELAY_MS: AtomicU64 = AtomicU64::new(0);

const MAX_STREAM_FAILURES: u32 = 6;

#[derive(Clone)]
pub struct GameContext {
    pub client: Arc<LichessClient>,
    pub engine: Arc<SearchEngine>,
    pub cfg: Arc<Config>,
    /// Lower-cased account id of the bot.
    pub my_id: String,
}

#[derive(Debug, PartialEq)]
pub enum GameOutcome {
    Finished,
    Abandoned(String),
}

// ---------------------------------------------------------------------------
// Pondering
// ---------------------------------------------------------------------------

/// Background search on the opponent's time.
///
/// Step 1: a quick depth-6 search predicts the opponent's reply.
/// Step 2: search our answer to that reply to warm the shared transposition
/// table. `SearchEngine::search` takes a lock and *resets* its stop flag on
/// entry, so a plain `engine.stop()` can be lost if it lands just before the
/// ponder thread enters `search()` (the old Python bot could block ~20 s that
/// way). `stop()` therefore invalidates the generation first and then keeps
/// signalling until the thread has actually exited.
pub struct Pondering {
    engine: Arc<SearchEngine>,
    max_ms: u64,
    generation: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl Pondering {
    pub fn new(engine: Arc<SearchEngine>, max_ms: u64) -> Self {
        Pondering {
            engine,
            max_ms,
            generation: Arc::new(AtomicU64::new(0)),
            handle: None,
        }
    }

    /// `board` is the position after *our* move (opponent to move).
    pub fn start(&mut self, board: BoardState) {
        self.stop();
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let gen = Arc::clone(&self.generation);
        let engine = Arc::clone(&self.engine);
        let max_ms = self.max_ms;
        match thread::Builder::new()
            .name("ponder".into())
            .stack_size(THREAD_STACK)
            .spawn(move || ponder_worker(engine, gen, my_gen, board, max_ms))
        {
            Ok(h) => self.handle = Some(h),
            Err(e) => eprintln!("warning: could not start pondering thread: {e}"),
        }
    }

    /// Blocks until no ponder search is running. After this returns it is safe
    /// to call `engine.search()` and it will not wait on a ponder search.
    pub fn stop(&mut self) {
        if let Some(h) = self.handle.take() {
            self.generation.fetch_add(1, Ordering::SeqCst);
            while !h.is_finished() {
                self.engine.stop();
                thread::sleep(Duration::from_millis(1));
            }
            let _ = h.join();
        }
    }

    pub fn is_active(&self) -> bool {
        self.handle.as_ref().map_or(false, |h| !h.is_finished())
    }
}

impl Drop for Pondering {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ponder_worker(
    engine: Arc<SearchEngine>,
    gen: Arc<AtomicU64>,
    my_gen: u64,
    after_our_move: BoardState,
    max_ms: u64,
) {
    let current = || gen.load(Ordering::SeqCst) == my_gen;
    if !current() {
        return;
    }

    // Step 1: predict the opponent's reply.
    let predicted = engine.search(after_our_move.clone(), 6, None);
    if !current() {
        return;
    }
    let Some(opp_move) = predicted.best_move else {
        return;
    };

    // Step 2: search our reply to the predicted move.
    let mut next = after_our_move;
    next.make_move(&opp_move);
    if !current() || MoveGenerator::generate_legal_moves(&next).is_empty() {
        return;
    }
    #[cfg(test)]
    thread::sleep(Duration::from_millis(
        PONDER_PRE_SEARCH_DELAY_MS.load(Ordering::SeqCst),
    ));
    engine.search(next, 64, Some(max_ms));
}

// ---------------------------------------------------------------------------
// Game loop
// ---------------------------------------------------------------------------

enum Flow {
    Continue,
    GameOver,
    Fatal(String),
}

enum StreamEnd {
    GameOver,
    Disconnected,
    Fatal(String),
}

enum SendResult {
    Sent,
    /// Server said no (illegal / not our turn / game over). Retrying is useless.
    Rejected(String),
    /// Could not reach the server after retries.
    NetworkFailure,
}

struct GameSession<'a> {
    ctx: &'a GameContext,
    game_id: String,
    /// Starting position of the game (from `initialFen`).
    base: BoardState,
    /// Current position, rebuilt from `base` + move list, so it carries the
    /// full repetition history the search needs for draw detection.
    board: BoardState,
    my_color: Option<Color>,
    started: bool,
    /// Ply count at which we last sent a move. Lichess re-sends `gameState`
    /// for things like draw offers; without this we'd answer twice.
    last_acted_ply: Option<usize>,
    events_seen: u64,
    ponder: Pondering,
}

pub fn play_game(ctx: &GameContext, game_id: &str) -> GameOutcome {
    let mut session = GameSession::new(ctx, game_id);
    let mut failures: u32 = 0;

    loop {
        let wait = match ctx.client.open_game_stream(game_id) {
            Ok(mut stream) => {
                let before = session.events_seen;
                let end = session.run_stream(&mut stream);
                match end {
                    StreamEnd::GameOver => return GameOutcome::Finished,
                    StreamEnd::Fatal(msg) => return GameOutcome::Abandoned(msg),
                    StreamEnd::Disconnected => {
                        if session.events_seen > before {
                            failures = 0;
                        } else {
                            failures += 1;
                        }
                        eprintln!("[{game_id}] game stream dropped, reconnecting...");
                        Duration::from_secs(1)
                    }
                }
            }
            Err(ApiError::Status(code @ (403 | 404), body)) => {
                return GameOutcome::Abandoned(format!("HTTP {code}: {body}"));
            }
            Err(ApiError::Status(429, _)) => {
                failures += 1;
                eprintln!("[{game_id}] rate limited (429), waiting 60s");
                Duration::from_secs(60)
            }
            Err(e) => {
                failures += 1;
                eprintln!("[{game_id}] cannot open game stream: {e}");
                Duration::from_secs((1u64 << failures.min(4)).min(10))
            }
        };
        if failures >= MAX_STREAM_FAILURES {
            return GameOutcome::Abandoned("game stream kept failing".into());
        }
        thread::sleep(wait);
    }
}

impl<'a> GameSession<'a> {
    fn new(ctx: &'a GameContext, game_id: &str) -> Self {
        GameSession {
            ctx,
            game_id: game_id.to_string(),
            base: BoardState::default(),
            board: BoardState::default(),
            my_color: None,
            started: false,
            last_acted_ply: None,
            events_seen: 0,
            ponder: Pondering::new(Arc::clone(&ctx.engine), ctx.cfg.ponder_max_ms),
        }
    }

    fn run_stream(&mut self, stream: &mut NdjsonStream) -> StreamEnd {
        loop {
            match stream.next_item() {
                None => return StreamEnd::Disconnected,
                Some(Err(e)) => {
                    eprintln!("[{}] stream read error: {e}", self.game_id);
                    return StreamEnd::Disconnected;
                }
                Some(Ok(StreamItem::KeepAlive)) => {}
                Some(Ok(StreamItem::Event(ev))) => {
                    self.events_seen += 1;
                    match self.handle_event(&ev) {
                        Flow::Continue => {}
                        Flow::GameOver => return StreamEnd::GameOver,
                        Flow::Fatal(m) => return StreamEnd::Fatal(m),
                    }
                }
            }
        }
    }

    fn handle_event(&mut self, ev: &Value) -> Flow {
        match ev["type"].as_str() {
            Some("gameFull") => self.on_game_full(ev),
            Some("gameState") => self.on_state(ev),
            Some("chatLine") => {
                println!(
                    "[{}] chat {}: {}",
                    self.game_id,
                    ev["username"].as_str().unwrap_or("?"),
                    ev["text"].as_str().unwrap_or("")
                );
                Flow::Continue
            }
            Some("opponentGone") => {
                println!("[{}] opponent disconnected", self.game_id);
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }

    fn on_game_full(&mut self, ev: &Value) -> Flow {
        // Reconnects re-send gameFull; make sure no ponder search is running
        // before touching engine state.
        self.ponder.stop();

        // Honour initialFen (the old Python bot rebuilt every position from the
        // standard start position and silently broke on custom starts).
        let initial = ev["initialFen"].as_str().unwrap_or("startpos");
        let fen = if initial == "startpos" { START_FEN } else { initial };
        self.base = match BoardState::from_fen(fen) {
            Ok(b) => b,
            Err(e) => return Flow::Fatal(format!("bad initialFen {fen:?}: {e}")),
        };

        let variant = ev["variant"]["key"].as_str().unwrap_or("standard");
        if variant != "standard" && variant != "fromPosition" {
            return Flow::Fatal(format!("unsupported variant {variant}"));
        }

        if !self.started {
            self.ctx.engine.new_game();
            self.started = true;
        }

        let white_id = ev["white"]["id"].as_str().unwrap_or("").to_lowercase();
        let black_id = ev["black"]["id"].as_str().unwrap_or("").to_lowercase();
        self.my_color = if white_id == self.ctx.my_id {
            Some(Color::White)
        } else if black_id == self.ctx.my_id {
            Some(Color::Black)
        } else {
            return Flow::Fatal("this account is neither white nor black".into());
        };
        self.last_acted_ply = None;

        println!(
            "[{}] playing as {} | white: {} ({}) | black: {} ({})",
            self.game_id,
            if self.my_color == Some(Color::White) { "WHITE" } else { "BLACK" },
            ev["white"]["name"].as_str().unwrap_or("?"),
            ev["white"]["rating"],
            ev["black"]["name"].as_str().unwrap_or("?"),
            ev["black"]["rating"],
        );

        self.on_state(&ev["state"])
    }

    fn on_state(&mut self, state: &Value) -> Flow {
        let status = state["status"].as_str().unwrap_or("started");
        if status != "started" && status != "created" {
            self.ponder.stop();
            println!(
                "[{}] game finished: {status}{}",
                self.game_id,
                state["winner"]
                    .as_str()
                    .map(|w| format!(", winner {w}"))
                    .unwrap_or_default()
            );
            return Flow::GameOver;
        }

        let Some(my_color) = self.my_color else {
            return Flow::Continue; // gameState before gameFull: ignore
        };

        let ply = match self.rebuild_board(state["moves"].as_str().unwrap_or("")) {
            Ok(n) => n,
            Err(e) => return Flow::Fatal(e),
        };

        if self.board.side_to_move != my_color {
            return Flow::Continue; // our own move echoed back, or opponent thinking
        }
        if self.last_acted_ply == Some(ply) {
            return Flow::Continue; // duplicate state (e.g. draw offer)
        }
        self.take_turn(state, ply)
    }

    /// Rebuild the position from the initial FEN and the full move list.
    fn rebuild_board(&mut self, moves: &str) -> Result<usize, String> {
        let mut b = self.base.clone();
        let mut n = 0usize;
        for m in moves.split_whitespace() {
            b.make_move_uci(m)
                .map_err(|e| format!("cannot apply move #{} {m:?}: {e}", n + 1))?;
            n += 1;
        }
        self.board = b;
        Ok(n)
    }

    fn take_turn(&mut self, state: &Value, ply: usize) -> Flow {
        let legal = MoveGenerator::generate_legal_moves(&self.board);
        // NB: do not use `board.is_game_over()` here. It also reports threefold
        // repetition / 50-move draws, which Lichess does NOT end automatically;
        // refusing to move there would lose on time.
        if legal.is_empty() {
            return Flow::Continue; // mate/stalemate; Lichess sends the final state
        }

        self.ponder.stop();

        let white = self.my_color == Some(Color::White);
        let (tkey, ikey) = if white { ("wtime", "winc") } else { ("btime", "binc") };
        let my_time = state[tkey].as_u64().unwrap_or(60_000).min(10_800_000);
        let my_inc = state[ikey].as_u64().unwrap_or(0).min(10_800_000);

        let alloc = timing::move_time_ms(my_time, my_inc, ply, self.ctx.cfg.move_overhead_ms);

        let t0 = Instant::now();
        let result = self
            .ctx
            .engine
            .search(self.board.clone(), self.ctx.cfg.search_depth, Some(alloc));
        let elapsed = t0.elapsed();

        // Never send something the engine could not have generated.
        let mv = match result.best_move {
            Some(m) if legal.contains(&m) => m,
            other => {
                eprintln!(
                    "[{}] WARNING: engine returned {:?}; falling back to first legal move",
                    self.game_id,
                    other.map(|m| m.to_uci())
                );
                legal[0]
            }
        };
        let uci = mv.to_uci();

        let secs = elapsed.as_secs_f64();
        let nps = if secs > 0.0 { result.nodes as f64 / secs } else { 0.0 };
        println!(
            "[{}] ply {:>3}: {:<6} score {:>+6}cp | {} nodes | {:.2}s (budget {}ms) | {:.2} Mnps | clock {:.1}s",
            self.game_id,
            ply + 1,
            uci,
            result.score,
            result.nodes,
            secs,
            alloc,
            nps / 1e6,
            my_time as f64 / 1000.0
        );

        match self.send_move(&uci) {
            SendResult::Sent => {
                self.last_acted_ply = Some(ply);
                if self.ctx.cfg.ponder {
                    let mut after = self.board.clone();
                    after.make_move(&mv);
                    self.ponder.start(after);
                }
                Flow::Continue
            }
            SendResult::Rejected(why) => {
                eprintln!("[{}] move {uci} rejected: {why}", self.game_id);
                self.last_acted_ply = Some(ply); // don't hot-loop on the same state
                Flow::Continue
            }
            SendResult::NetworkFailure => {
                // Leave last_acted_ply unset: a stream reconnect (gameFull)
                // will re-run this turn.
                eprintln!("[{}] could not deliver move {uci}", self.game_id);
                Flow::Continue
            }
        }
    }

    fn send_move(&self, uci: &str) -> SendResult {
        const ATTEMPTS: u32 = 4;
        for attempt in 0..ATTEMPTS {
            match self.ctx.client.make_move(&self.game_id, uci) {
                Ok(()) => return SendResult::Sent,
                Err(ApiError::Status(code, body)) if code == 429 || code >= 500 => {
                    eprintln!("[{}] move POST got HTTP {code}: {body}", self.game_id);
                }
                Err(ApiError::Status(_, body)) => return SendResult::Rejected(body),
                Err(ApiError::BadInput(e)) => return SendResult::Rejected(e),
                Err(ApiError::Transport(e)) => {
                    eprintln!("[{}] move POST failed: {e}", self.game_id);
                }
            }
            if attempt + 1 < ATTEMPTS {
                thread::sleep(Duration::from_millis(200 << attempt));
            }
        }
        SendResult::NetworkFailure
    }
}
