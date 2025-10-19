# 🦀 Rust Chess Engine

A high-performance chess engine written in Rust with Python bindings, designed to play on Lichess and other UCI-compatible chess platforms.

## ✨ Features

- **Blazing Fast**: Written in Rust for maximum performance (~1-2M nodes/second)
- **Strong Play**: Advanced search algorithms with move ordering, transposition tables, and evaluation
- **Opening Book**: Built-in opening repertoire for solid opening play
- **Lichess Integration**: Ready-to-use bot that connects directly to Lichess
- **UCI Compatible**: Works with any UCI chess GUI (Arena, Cutechess, PyChess)
- **Python Bindings**: Easy to use from Python via PyO3

## 🎯 Technical Features

### Search
- **Negamax** with alpha-beta pruning
- **Iterative Deepening** for time management
- **Aspiration Windows** for faster re-searches
- **Transposition Table** to avoid re-searching positions
- **Move Ordering**:
  - Hash move from TT
  - MVV-LVA (Most Valuable Victim - Least Valuable Attacker)
  - Killer move heuristic
  - History heuristic
- **Late Move Reductions (LMR)** for pruning unlikely moves
- **Null Move Pruning** for forward pruning
- **Quiescence Search** to handle tactical positions
- **Check Extensions** to avoid horizon effect
- **Mate Distance Pruning**

### Evaluation
- **Material counting** with piece values
- **Piece-Square Tables** for positional play (separate middlegame/endgame)
- **Pawn structure** evaluation (doubled, isolated, passed pawns)
- **King safety** assessment
- **Mobility** bonus
- **Special bonuses**: Bishop pair, rook on open files, knight outposts
- **Tapered evaluation** (smooth transition from middlegame to endgame)

### Move Generation
- **Bitboard** representation for fast move generation
- **Magic bitboards** for sliding piece attacks
- **Legal move verification**
- **Special moves**: Castling, en passant, promotions

## 🚀 Quick Start

### Prerequisites

- **Rust** (1.70+): Install from [rustup.rs](https://rustup.rs)
- **Python** (3.8+): With pip
- **maturin**: For building Python bindings

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/rust-chess-engine.git
cd rust-chess-engine

# Install maturin
pip install maturin

# Build the engine (takes a few minutes)
maturin develop --release

# Verify installation
python3 -c "import chess_engine; print('✓ Engine loaded!')"
```

### Optimized Build (Faster!)

```bash
# Build with native CPU optimizations
RUSTFLAGS="-C target-cpu=native" maturin develop --release
```

## 🎮 Usage

### As a Lichess Bot

1. **Create a Lichess Bot Account**:
   - Go to https://lichess.org/account/preferences/bot
   - Click "Upgrade to Bot Account"

2. **Get API Token**:
   - Visit https://lichess.org/account/oauth/token/create
   - Select scopes: `bot:play`, `challenge:read`, `challenge:write`
   - Copy your token

3. **Run the Bot**:
```bash
python3 lichess_bot.py YOUR_LICHESS_TOKEN
```

Or set as environment variable:
```bash
export LICHESS_TOKEN="your_token_here"
python3 lichess_bot.py
```

### From Python

```python
import chess_engine

# Create engine
engine = chess_engine.PyChessEngine(threads=4)

# Search a position
result = engine.search(
    fen="rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    depth=12,
    time_ms=5000  # Optional time limit
)

print(f"Best move: {result['move']}")
print(f"Score: {result['score']} centipawns")
print(f"Nodes: {result['nodes']:,}")
```

### As UCI Engine

```bash
# Compile UCI binary
cargo build --release --bin uci

# Run UCI interface
./target/release/uci
```

Then use with any UCI-compatible chess GUI:
- Arena Chess GUI
- Cutechess
- PyChess
- Scid vs. PC

## 📊 Performance

- **Search Speed**: 1-2 million nodes/second (M1/M2 Mac)
- **Typical Depth**: 10-14 ply in middlegame positions
- **Time Management**: Adaptive based on game phase and time control
- **Opening Book**: ~100 positions covering major openings

## 🏗️ Project Structure

```
rust-chess-engine/
├── src/
│   ├── lib.rs              # Python bindings (PyO3)
│   ├── board.rs            # Board representation & FEN parsing
│   ├── bitboard.rs         # Bitboard operations
│   ├── movegen.rs          # Move generation
│   ├── search.rs           # Search algorithms
│   ├── eval.rs             # Position evaluation
│   ├── zobrist.rs          # Zobrist hashing
│   ├── opening_book.rs     # Opening book
│   └── uci.rs              # UCI protocol implementation
├── Cargo.toml              # Rust dependencies
├── lichess_bot.py          # Lichess bot integration
└── README.md
```

## 🎯 Strength

Estimated Rating: **~2000-2200 ELO** (Lichess Rapid)

The engine plays solidly in:
- ✅ Tactical positions
- ✅ Endgames
- ✅ Standard openings
- ⚠️ Still learning complex strategic positions

## 🔧 Configuration

### Hash Table Size
```python
engine = chess_engine.PyChessEngine(threads=4)
engine.set_hash_size(512)  # 512 MB (default: 256 MB)
```

### Thread Count
```python
engine = chess_engine.PyChessEngine(threads=8)  # Use 8 threads
```

### Search Parameters
Modify constants in `search.rs`:
```rust
pub const MATE_SCORE: i32 = 900000;  // Mate detection threshold
```

## 🛠️ Development

### Run Tests
```bash
cargo test
```

### Benchmarks
```bash
cargo bench
```

### Debug Build (faster compilation)
```bash
maturin develop
```

### Profile with Flamegraph
```bash
cargo install flamegraph
cargo flamegraph --bench search_bench
```

## 📈 Future Improvements

- [ ] **Neural Network Evaluation** (NNUE)
- [ ] **Lazy SMP** for parallel search
- [ ] **Singular Extensions**
- [ ] **More sophisticated time management**
- [ ] **Extended opening book**
- [ ] **Endgame tablebases** (Syzygy)
- [ ] **Contempt factor** for anti-draw play
- [ ] **Multi-PV support**
- [ ] **Analysis mode**

## 🤝 Contributing

Contributions are welcome! Areas for improvement:

1. **Search Enhancements**: Implement new pruning techniques
2. **Evaluation Tuning**: Optimize piece-square tables and weights
3. **Opening Book**: Add more positions and variations
4. **Bug Fixes**: Report and fix any issues
5. **Documentation**: Improve code comments and docs

## 📝 License

MIT License - See LICENSE file for details

## 🙏 Acknowledgments

Inspired by:
- [Stockfish](https://github.com/official-stockfish/Stockfish) - The world's strongest chess engine
- [Chess Programming Wiki](https://www.chessprogramming.org/) - Comprehensive resource
- [PyO3](https://github.com/PyO3/pyo3) - Rust ↔ Python bindings

## 📧 Contact

- **Author**: Your Name
- **GitHub**: [@yourusername](https://github.com/yourusername)
- **Lichess**: [@YourBotUsername](https://lichess.org/@/YourBotUsername)

---

**Made with 🦀 Rust and ❤️ for Chess**

*"The beauty of a move lies not in its appearance but in the thought behind it."*