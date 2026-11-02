//! Minimal blocking Lichess Bot API client (ureq + rustls).

use serde_json::Value;
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

#[derive(Debug)]
pub enum ApiError {
    /// The server answered with a non-2xx status.
    Status(u16, String),
    /// Connection / TLS / timeout problem.
    Transport(String),
    /// We refused to build the request (malformed id / move).
    BadInput(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Status(code, body) => write!(f, "HTTP {code}: {body}"),
            ApiError::Transport(e) => write!(f, "network error: {e}"),
            ApiError::BadInput(e) => write!(f, "bad input: {e}"),
        }
    }
}

impl std::error::Error for ApiError {}

fn map_err(e: ureq::Error) -> ApiError {
    match e {
        ureq::Error::Status(code, resp) => {
            let mut body = resp.into_string().unwrap_or_default();
            body.truncate(300);
            ApiError::Status(code, body)
        }
        ureq::Error::Transport(t) => ApiError::Transport(t.to_string()),
    }
}

/// Ids come from the network and end up inside URL paths; only allow what
/// Lichess actually uses.
fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn valid_uci(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 4 || b.len() == 5)
        && (b'a'..=b'h').contains(&b[0])
        && (b'1'..=b'8').contains(&b[1])
        && (b'a'..=b'h').contains(&b[2])
        && (b'1'..=b'8').contains(&b[3])
        && (b.len() == 4 || matches!(b[4], b'q' | b'r' | b'b' | b'n'))
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub username: String,
}

pub struct LichessClient {
    base: String,
    auth: String,
    /// Short request/response calls: hard overall timeout.
    api: ureq::Agent,
    /// Long-lived NDJSON streams: only a per-read timeout. Lichess sends a
    /// keep-alive newline every few seconds, so silence means a dead socket.
    stream: ureq::Agent,
}

impl LichessClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        let api = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .user_agent("RustChessEngine-lichess-bot")
            .build();
        let stream = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(90))
            .user_agent("RustChessEngine-lichess-bot")
            .build();
        LichessClient {
            base: base_url.trim_end_matches('/').to_string(),
            auth: format!("Bearer {token}"),
            api,
            stream,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub fn account(&self) -> Result<Account, ApiError> {
        let body = self
            .api
            .get(&self.url("/api/account"))
            .set("Authorization", &self.auth)
            .call()
            .map_err(map_err)?
            .into_string()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| ApiError::Status(200, format!("account response is not JSON: {e}")))?;
        let id = v["id"].as_str().unwrap_or("").to_lowercase();
        if id.is_empty() {
            return Err(ApiError::Status(200, "account response had no id".into()));
        }
        Ok(Account {
            id,
            username: v["username"].as_str().unwrap_or("?").to_string(),
        })
    }

    fn open_stream(&self, path: &str) -> Result<NdjsonStream, ApiError> {
        let resp = self
            .stream
            .get(&self.url(path))
            .set("Authorization", &self.auth)
            .set("Accept", "application/x-ndjson")
            .call()
            .map_err(map_err)?;
        Ok(NdjsonStream::from_reader(resp.into_reader()))
    }

    pub fn open_event_stream(&self) -> Result<NdjsonStream, ApiError> {
        self.open_stream("/api/stream/event")
    }

    pub fn open_game_stream(&self, game_id: &str) -> Result<NdjsonStream, ApiError> {
        if !valid_id(game_id) {
            return Err(ApiError::BadInput(format!("game id {game_id:?}")));
        }
        self.open_stream(&format!("/api/bot/game/stream/{game_id}"))
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<(), ApiError> {
        let req = self
            .api
            .post(&self.url(path))
            .set("Authorization", &self.auth);
        let res = if form.is_empty() {
            req.send_bytes(&[])
        } else {
            req.send_form(form)
        };
        res.map(|_| ()).map_err(map_err)
    }

    pub fn accept_challenge(&self, id: &str) -> Result<(), ApiError> {
        if !valid_id(id) {
            return Err(ApiError::BadInput(format!("challenge id {id:?}")));
        }
        self.post(&format!("/api/challenge/{id}/accept"), &[])
    }

    pub fn decline_challenge(&self, id: &str, reason: &str) -> Result<(), ApiError> {
        if !valid_id(id) {
            return Err(ApiError::BadInput(format!("challenge id {id:?}")));
        }
        self.post(&format!("/api/challenge/{id}/decline"), &[("reason", reason)])
    }

    pub fn make_move(&self, game_id: &str, uci: &str) -> Result<(), ApiError> {
        if !valid_id(game_id) {
            return Err(ApiError::BadInput(format!("game id {game_id:?}")));
        }
        if !valid_uci(uci) {
            return Err(ApiError::BadInput(format!("move {uci:?}")));
        }
        self.post(&format!("/api/bot/game/{game_id}/move/{uci}"), &[])
    }
}

pub enum StreamItem {
    Event(Value),
    /// Blank line (Lichess heartbeat) or a line we could not parse.
    KeepAlive,
}

/// Newline-delimited JSON stream reader.
pub struct NdjsonStream {
    reader: BufReader<Box<dyn Read + Send + Sync>>,
    buf: String,
}

impl NdjsonStream {
    pub fn from_reader<R: Read + Send + Sync + 'static>(r: R) -> Self {
        NdjsonStream {
            reader: BufReader::new(Box::new(r)),
            buf: String::new(),
        }
    }

    /// `None` = clean end of stream. `Some(Err)` = read error / timeout.
    pub fn next_item(&mut self) -> Option<std::io::Result<StreamItem>> {
        self.buf.clear();
        match self.reader.read_line(&mut self.buf) {
            Ok(0) => None,
            Ok(_) => {
                let line = self.buf.trim();
                if line.is_empty() {
                    return Some(Ok(StreamItem::KeepAlive));
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(v) => Some(Ok(StreamItem::Event(v))),
                    Err(e) => {
                        eprintln!("warning: ignoring unparseable stream line ({e})");
                        Some(Ok(StreamItem::KeepAlive))
                    }
                }
            }
            Err(e) => Some(Err(e)),
        }
    }
}
