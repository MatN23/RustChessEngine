//! In-process fake Lichess server (test only).
//!
//! Speaks the parts of the Bot API the client uses: NDJSON event/game streams
//! with keep-alive newlines, `gameFull`/`gameState`, challenge accept/decline,
//! and move POSTs that are rejected with HTTP 400 when illegal or out of turn.
//! The "human" opponent is played by a second `SearchEngine` at low depth, and
//! every bot move is validated against an independent move generator.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::board::{BoardState, Color};
use crate::movegen::{Move, MoveGenerator};
use crate::search::SearchEngine;

pub const TOKEN: &str = "testtoken";
pub const BOT_ID: &str = "testbot";

#[derive(Clone)]
pub struct MockOptions {
    pub bot_white: bool,
    pub initial_fen: Option<String>,
    /// UCI moves already played when the bot first sees the game.
    pub preset_moves: Vec<String>,
    /// Game is declared drawn once this many plies were played.
    pub max_plies: usize,
    /// Delay before the opponent answers (gives pondering time to run).
    pub opp_think_ms: u64,
    pub opp_depth: u8,
    /// Re-send the same gameState after each opponent move (what Lichess does
    /// for draw offers etc.).
    pub duplicate_states: bool,
    /// Close the game stream right after the bot's k-th accepted move.
    pub drop_stream_after_bot_move: Option<usize>,
    /// Challenge events (`{"type":"challenge", ...}`) sent on event-stream connect.
    pub challenges: Vec<Value>,
    pub clock_ms: u64,
    /// Usernames returned by GET /api/bot/online.
    pub online_bots: Vec<String>,
    /// HTTP status for POST /api/challenge/{user} (200 = created).
    pub create_status: u16,
}

impl Default for MockOptions {
    fn default() -> Self {
        MockOptions {
            bot_white: true,
            initial_fen: None,
            preset_moves: vec![],
            max_plies: 24,
            opp_think_ms: 0,
            opp_depth: 3,
            duplicate_states: false,
            drop_stream_after_bot_move: None,
            challenges: vec![],
            clock_ms: 20_000,
            online_bots: vec![],
            create_status: 200,
        }
    }
}

pub struct MockGame {
    pub id: String,
    pub initial_fen: String,
    pub board: BoardState,
    pub moves: Vec<String>,
    pub bot_color: Color,
    pub status: String,
    pub winner: Option<String>,
    pub finished: bool,
    line_tx: Option<Sender<Option<String>>>,
    opp: Arc<SearchEngine>,
}

pub struct MockState {
    pub opts: MockOptions,
    challenge_ids: HashSet<String>,
    event_tx: Option<Sender<Option<String>>>,
    pub accepted: Vec<String>,
    pub declined: Vec<(String, String)>,
    /// (username, form body) of every POST /api/challenge/{user}.
    pub created: Vec<(String, String)>,
    pub cancelled: Vec<String>,
    pub bot_moves: Vec<String>,
    pub rejected: Vec<String>,
    pub game_streams_opened: usize,
    pub unauthorized: usize,
    dropped_once: bool,
    pub game: Option<MockGame>,
}

pub struct MockServer {
    pub url: String,
    pub state: Arc<Mutex<MockState>>,
    stop: Arc<AtomicBool>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub fn challenge_event(id: &str, variant: &str, speed: &str, tc_type: &str) -> Value {
    json!({
        "type": "challenge",
        "challenge": {
            "id": id,
            "status": "created",
            "challenger": {"id": "human1", "name": "Human1", "rating": 1500},
            "destUser": {"id": BOT_ID, "name": "TestBot"},
            "variant": {"key": variant, "name": variant},
            "rated": false,
            "speed": speed,
            "timeControl": {"type": tc_type, "limit": 300, "increment": 0, "show": "5+0"}
        }
    })
}

impl MockServer {
    pub fn start(opts: MockOptions) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        let challenge_ids = opts
            .challenges
            .iter()
            .filter_map(|c| c["challenge"]["id"].as_str().map(str::to_string))
            .collect();
        let state = Arc::new(Mutex::new(MockState {
            opts,
            challenge_ids,
            event_tx: None,
            accepted: vec![],
            declined: vec![],
            created: vec![],
            cancelled: vec![],
            bot_moves: vec![],
            rejected: vec![],
            game_streams_opened: 0,
            unauthorized: 0,
            dropped_once: false,
            game: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let (st, sp) = (Arc::clone(&state), Arc::clone(&stop));
        thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let (st, sp) = (Arc::clone(&st), Arc::clone(&sp));
                        thread::spawn(move || handle_conn(conn, st, sp));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        MockServer {
            url: format!("http://{addr}"),
            state,
            stop,
        }
    }

    pub fn game_finished(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .game
            .as_ref()
            .map_or(false, |g| g.finished)
    }
}

// ---------------------------------------------------------------------------
// HTTP plumbing
// ---------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    auth: String,
    body: String,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let head_end = loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 65_536 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let mut auth = String::new();
    let mut content_length = 0usize;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "authorization" => auth = v.trim().to_string(),
                "content-length" => content_length = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    while buf.len() < head_end + content_length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let end = (head_end + content_length).min(buf.len());
    Some(Request {
        method,
        path,
        auth,
        body: String::from_utf8_lossy(&buf[head_end..end]).to_string(),
    })
}

fn respond_json(stream: &mut TcpStream, code: u16, body: &str) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        429 => "Too Many Requests",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

fn begin_stream(stream: &mut TcpStream) -> bool {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
        )
        .and_then(|_| stream.flush())
        .is_ok()
}

/// Pump lines to the client; blank keep-alive line on idle. `None` closes.
fn stream_loop(stream: &mut TcpStream, rx: Receiver<Option<String>>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Some(line)) => {
                if writeln!(stream, "{line}").and_then(|_| stream.flush()).is_err() {
                    return;
                }
            }
            Ok(None) => return,
            Err(RecvTimeoutError::Timeout) => {
                if stream.write_all(b"\n").and_then(|_| stream.flush()).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn handle_conn(mut stream: TcpStream, state: Arc<Mutex<MockState>>, stop: Arc<AtomicBool>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(req) = read_request(&mut stream) else {
        return;
    };

    if req.auth != format!("Bearer {TOKEN}") {
        state.lock().unwrap().unauthorized += 1;
        respond_json(&mut stream, 401, r#"{"error":"No such token"}"#);
        return;
    }

    let path = req.path.as_str();
    match (req.method.as_str(), path) {
        ("GET", "/api/account") => {
            respond_json(
                &mut stream,
                200,
                &json!({"id": BOT_ID, "username": "TestBot"}).to_string(),
            );
        }
        ("GET", "/api/stream/event") => {
            let (tx, rx) = channel();
            {
                let mut st = state.lock().unwrap();
                for c in st.opts.challenges.clone() {
                    let _ = tx.send(Some(c.to_string()));
                }
                st.event_tx = Some(tx);
            }
            if begin_stream(&mut stream) {
                stream_loop(&mut stream, rx, &stop);
            }
        }
        ("GET", p) if p.starts_with("/api/bot/game/stream/") => {
            let id = p.trim_start_matches("/api/bot/game/stream/").to_string();
            serve_game_stream(&mut stream, &state, &stop, &id);
        }
        ("POST", p) if p.starts_with("/api/challenge/") && p.ends_with("/accept") => {
            let id = p
                .trim_start_matches("/api/challenge/")
                .trim_end_matches("/accept");
            let (code, body) = handle_accept(&state, id);
            respond_json(&mut stream, code, &body);
        }
        ("POST", p) if p.starts_with("/api/challenge/") && p.ends_with("/decline") => {
            let id = p
                .trim_start_matches("/api/challenge/")
                .trim_end_matches("/decline");
            let reason = req
                .body
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == "reason")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            state
                .lock()
                .unwrap()
                .declined
                .push((id.to_string(), reason));
            respond_json(&mut stream, 200, r#"{"ok":true}"#);
        }
        ("POST", p) if p.starts_with("/api/challenge/") && p.ends_with("/cancel") => {
            let id = p
                .trim_start_matches("/api/challenge/")
                .trim_end_matches("/cancel");
            state.lock().unwrap().cancelled.push(id.to_string());
            respond_json(&mut stream, 200, r#"{"ok":true}"#);
        }
        ("POST", p) if p.starts_with("/api/challenge/") => {
            let user = p.trim_start_matches("/api/challenge/").to_string();
            let (status, n) = {
                let mut st = state.lock().unwrap();
                st.created.push((user, req.body.clone()));
                (st.opts.create_status, st.created.len())
            };
            if status == 200 {
                respond_json(&mut stream, 200, &json!({"challenge": {"id": format!("mockch{n:02}")}}).to_string());
            } else {
                respond_json(&mut stream, status, r#"{"error":"nope"}"#);
            }
        }
        ("GET", p) if p.starts_with("/api/bot/online") => {
            let lines: Vec<String> = state
                .lock()
                .unwrap()
                .opts
                .online_bots
                .iter()
                .map(|n| json!({"id": n.to_lowercase(), "username": n}).to_string())
                .collect();
            let body = lines.join("\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
        ("POST", p) if p.starts_with("/api/bot/game/") && p.contains("/move/") => {
            let rest = p.trim_start_matches("/api/bot/game/");
            let (id, uci) = rest.split_once("/move/").unwrap();
            let (code, body, followup) = handle_move(&state, id, uci);
            respond_json(&mut stream, code, &body);
            if followup {
                opponent_reply(&state);
            }
        }
        _ => respond_json(&mut stream, 404, r#"{"error":"not found"}"#),
    }
}

// ---------------------------------------------------------------------------
// Game logic
// ---------------------------------------------------------------------------

fn state_json(g: &MockGame, opts: &MockOptions) -> Value {
    let mut v = json!({
        "type": "gameState",
        "moves": g.moves.join(" "),
        "wtime": opts.clock_ms,
        "btime": opts.clock_ms,
        "winc": 0,
        "binc": 0,
        "status": g.status,
    });
    if let Some(w) = &g.winner {
        v["winner"] = json!(w);
    }
    v
}

fn game_full_json(g: &MockGame, opts: &MockOptions) -> Value {
    let bot = json!({"id": BOT_ID, "name": "TestBot", "rating": 2000});
    let human = json!({"id": "human1", "name": "Human1", "rating": 1500});
    let (white, black) = if g.bot_color == Color::White {
        (bot, human)
    } else {
        (human, bot)
    };
    json!({
        "type": "gameFull",
        "id": g.id,
        "variant": {"key": "standard", "name": "Standard"},
        "speed": "blitz",
        "rated": false,
        "white": white,
        "black": black,
        "initialFen": g.initial_fen,
        "state": state_json(g, opts),
    })
}

fn push(g: &MockGame, line: Value) {
    if let Some(tx) = &g.line_tx {
        let _ = tx.send(Some(line.to_string()));
    }
}

fn close_game_stream(g: &mut MockGame) {
    if let Some(tx) = g.line_tx.take() {
        let _ = tx.send(None);
    }
}

/// Update status/winner after a move. 3-fold repetition deliberately does not
/// end the game (Lichess does not end games on a threefold claim either).
fn check_game_end(g: &mut MockGame, max_plies: usize) {
    let legal = MoveGenerator::generate_legal_moves(&g.board);
    if legal.is_empty() {
        if g.board.is_in_check(g.board.side_to_move) {
            g.status = "mate".into();
            g.winner = Some(
                if g.board.side_to_move == Color::White { "black" } else { "white" }.into(),
            );
        } else {
            g.status = "stalemate".into();
        }
    } else if g.moves.len() >= max_plies {
        g.status = "draw".into();
    }
    if g.status != "started" {
        g.finished = true;
    }
}

/// Push the current state; if the game ended also close the stream.
fn publish(g: &mut MockGame, opts: &MockOptions, duplicate: bool) {
    let s = state_json(g, opts);
    push(g, s.clone());
    if duplicate && !g.finished {
        push(g, s);
    }
    if g.finished {
        close_game_stream(g);
    }
}

fn apply(g: &mut MockGame, mv: Move) {
    g.board.make_move(&mv);
    g.moves.push(mv.to_uci());
}

fn handle_accept(state: &Arc<Mutex<MockState>>, id: &str) -> (u16, String) {
    let mut guard = state.lock().unwrap();
    let st: &mut MockState = &mut guard;
    if !st.challenge_ids.contains(id) {
        return (404, r#"{"error":"unknown challenge"}"#.into());
    }
    st.accepted.push(id.to_string());
    if st.game.is_some() {
        return (200, r#"{"ok":true}"#.into());
    }

    let fen = st.opts.initial_fen.clone();
    let mut board = match &fen {
        Some(f) => BoardState::from_fen(f).expect("test FEN"),
        None => BoardState::default(),
    };
    let mut moves = vec![];
    for m in &st.opts.preset_moves {
        board.make_move_uci(m).expect("preset move legal");
        moves.push(m.clone());
    }
    let opp = Arc::new(SearchEngine::new(1));
    opp.set_hash_size(8);
    let bot_color = if st.opts.bot_white { Color::White } else { Color::Black };
    let mut g = MockGame {
        id: id.to_string(),
        initial_fen: fen.unwrap_or_else(|| "startpos".into()),
        board,
        moves,
        bot_color,
        status: "started".into(),
        winner: None,
        finished: false,
        line_tx: None,
        opp,
    };

    // Opponent moves first when the bot is not on move.
    if g.board.side_to_move != g.bot_color {
        let res = g.opp.search(g.board.clone(), st.opts.opp_depth, Some(300));
        let mv = res.best_move.expect("opp move");
        apply(&mut g, mv);
        check_game_end(&mut g, st.opts.max_plies);
    }

    let start = json!({"type": "gameStart", "game": {
        "gameId": id, "id": id, "fullId": format!("{id}abcd"),
        "color": if st.opts.bot_white {"white"} else {"black"},
        "hasMoved": false, "isMyTurn": g.board.side_to_move == g.bot_color,
    }});
    if let Some(tx) = &st.event_tx {
        let _ = tx.send(Some(start.to_string()));
    }
    st.game = Some(g);
    (200, r#"{"ok":true}"#.into())
}

fn serve_game_stream(
    stream: &mut TcpStream,
    state: &Arc<Mutex<MockState>>,
    stop: &AtomicBool,
    id: &str,
) {
    let (tx, rx) = channel();
    let full = {
        let mut guard = state.lock().unwrap();
        let st: &mut MockState = &mut guard;
        let Some(g) = st.game.as_mut().filter(|g| g.id == id) else {
            drop(guard);
            respond_json(stream, 404, r#"{"error":"no such game"}"#);
            return;
        };
        st.game_streams_opened += 1;
        let full = game_full_json(g, &st.opts);
        if g.finished {
            let _ = tx.send(None);
        } else {
            g.line_tx = Some(tx);
        }
        full
    };
    if !begin_stream(stream) {
        return;
    }
    if writeln!(stream, "{full}").and_then(|_| stream.flush()).is_err() {
        return;
    }
    stream_loop(stream, rx, stop);
}

/// Returns (http status, body, whether the opponent should now reply).
fn handle_move(state: &Arc<Mutex<MockState>>, id: &str, uci: &str) -> (u16, String, bool) {
    let mut guard = state.lock().unwrap();
    let st: &mut MockState = &mut guard;
    let Some(g) = st.game.as_mut().filter(|g| g.id == id) else {
        st.rejected.push(format!("unknown game {id}"));
        return (404, r#"{"error":"no such game"}"#.into(), false);
    };
    if g.finished {
        st.rejected.push(format!("game over, got {uci}"));
        return (400, r#"{"error":"Game already over"}"#.into(), false);
    }
    if g.board.side_to_move != g.bot_color {
        st.rejected.push(format!("not your turn: {uci}"));
        return (400, r#"{"error":"Not your turn"}"#.into(), false);
    }
    let legal = MoveGenerator::generate_legal_moves(&g.board);
    let Some(mv) = legal.iter().find(|m| m.to_uci() == uci).copied() else {
        st.rejected.push(format!("illegal: {uci} in {}", g.board.to_fen()));
        return (400, r#"{"error":"Illegal move"}"#.into(), false);
    };

    apply(g, mv);
    st.bot_moves.push(uci.to_string());
    check_game_end(g, st.opts.max_plies);
    publish(g, &st.opts, false);

    if let Some(k) = st.opts.drop_stream_after_bot_move {
        if !st.dropped_once && st.bot_moves.len() == k && !g.finished {
            st.dropped_once = true;
            close_game_stream(g);
        }
    }

    let followup = !g.finished;
    (200, r#"{"ok":true}"#.into(), followup)
}

fn opponent_reply(state: &Arc<Mutex<MockState>>) {
    let (think, depth, opp, board, nmoves) = {
        let st = state.lock().unwrap();
        let Some(g) = &st.game else { return };
        if g.finished || g.board.side_to_move == g.bot_color {
            return;
        }
        (
            st.opts.opp_think_ms,
            st.opts.opp_depth,
            Arc::clone(&g.opp),
            g.board.clone(),
            g.moves.len(),
        )
    };
    thread::sleep(Duration::from_millis(think));
    let res = opp.search(board.clone(), depth, Some(300));

    let mut guard = state.lock().unwrap();
    let st: &mut MockState = &mut guard;
    let Some(g) = st.game.as_mut() else { return };
    if g.finished || g.moves.len() != nmoves {
        return;
    }
    let mv = res
        .best_move
        .or_else(|| MoveGenerator::generate_legal_moves(&board).first().copied())
        .expect("opponent has a move");
    apply(g, mv);
    check_game_end(g, st.opts.max_plies);
    publish(g, &st.opts, st.opts.duplicate_states);
}
