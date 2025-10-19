use pyo3::prelude::*;

mod board;
mod bitboard;
mod movegen;
mod search;
mod eval;
mod zobrist;
mod opening_book;
mod uci;

use board::BoardState;
use search::SearchEngine;

#[pymodule]
fn chess_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyChessEngine>()?;
    m.add_class::<PyBoardState>()?;
    Ok(())
}

#[pyclass]
struct PyChessEngine {
    engine: SearchEngine,
}

#[pymethods]
impl PyChessEngine {
    #[new]
    #[pyo3(signature = (threads=None))]
    fn new(threads: Option<usize>) -> Self {
        PyChessEngine {
            engine: SearchEngine::new(threads.unwrap_or(4)),
        }
    }

    #[pyo3(signature = (fen, depth=None, time_ms=None))]
    fn search(
        &mut self,
        py: Python<'_>,
        fen: &str,
        depth: Option<u8>,
        time_ms: Option<u64>,
    ) -> PyResult<PyObject> {
        let board = BoardState::from_fen(fen)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?;
        
        let result = self.engine.search(
            board,
            depth.unwrap_or(64),
            time_ms,
        );

        let dict = pyo3::types::PyDict::new_bound(py);
        
        let move_str = result.best_move.map(|m| m.to_uci()).unwrap_or_else(|| "none".to_string());
        dict.set_item("move", move_str)?;
        dict.set_item("score", result.score)?;
        dict.set_item("nodes", result.nodes)?;
        
        Ok(dict.into())
    }

    fn new_game(&mut self) {
        self.engine.new_game();
    }

    fn set_threads(&mut self, threads: usize) {
        self.engine.set_threads(threads);
    }
    
    fn set_multi_pv(&mut self, count: usize) {
        self.engine.set_multi_pv(count);
    }
    
    fn set_hash_size(&mut self, size_mb: usize) {
        self.engine.set_hash_size(size_mb);
    }

    fn stop(&mut self) {
        self.engine.stop();
    }
}

#[pyclass]
struct PyBoardState {
    board: BoardState,
}

#[pymethods]
impl PyBoardState {
    #[new]
    #[pyo3(signature = (fen=None))]
    fn new(fen: Option<&str>) -> PyResult<Self> {
        let board = if let Some(fen_str) = fen {
            BoardState::from_fen(fen_str)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?
        } else {
            BoardState::default()
        };
        
        Ok(PyBoardState { board })
    }

    fn to_fen(&self) -> String {
        self.board.to_fen()
    }

    fn make_move(&mut self, uci: &str) -> PyResult<bool> {
        self.board.make_move_uci(uci)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))
    }

    fn is_game_over(&self) -> bool {
        self.board.is_game_over()
    }

    fn is_in_check(&self) -> bool {
        self.board.is_in_check(self.board.side_to_move)
    }
}