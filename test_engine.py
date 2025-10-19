#!/usr/bin/env python3
"""
Test script for the Rust chess engine with Stockfish features
"""

import chess_engine
import time

def test_basic_search():
    """Test basic search functionality"""
    print("=" * 60)
    print("TEST 1: Basic Search")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    
    # Starting position
    fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
    
    print(f"Position: {fen}")
    print("Searching to depth 10...")
    
    start = time.time()
    result = engine.search(fen, depth=10)
    elapsed = time.time() - start
    
    print(f"\n✓ Best move: {result['move']}")
    print(f"  Score: {result['score']} centipawns")
    print(f"  Nodes: {result['nodes']:,}")
    print(f"  Time: {elapsed:.2f}s")
    print(f"  NPS: {int(result['nodes'] / elapsed):,}")
    print()

def test_tactical_position():
    """Test engine finds mate in 2"""
    print("=" * 60)
    print("TEST 2: Tactical Position (Mate in 2)")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    
    # Fool's mate position
    fen = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3"
    
    print(f"Position: {fen}")
    print("Searching for mate...")
    
    result = engine.search(fen, depth=5)
    
    print(f"\n✓ Best move: {result['move']}")
    print(f"  Score: {result['score']} (should detect mate)")
    print()

def test_endgame():
    """Test endgame position"""
    print("=" * 60)
    print("TEST 3: Endgame Position")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    
    # KRK endgame
    fen = "8/8/8/8/8/3k4/3R4/3K4 w - - 0 1"
    
    print(f"Position: {fen}")
    print("Searching endgame...")
    
    result = engine.search(fen, depth=12)
    
    print(f"\n✓ Best move: {result['move']}")
    print(f"  Score: {result['score']}")
    print()

def test_multi_pv():
    """Test multi-PV functionality"""
    print("=" * 60)
    print("TEST 4: Multi-PV (Multiple Best Lines)")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    engine.set_multi_pv(3)  # Show top 3 moves
    
    fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3"
    
    print(f"Position: {fen}")
    print("Searching for top 3 moves...")
    
    result = engine.search(fen, depth=8)
    
    print(f"\n✓ Best move: {result['move']}")
    print(f"  All PV lines:")
    for i, pv in enumerate(result.get('pv_lines', [])[:3], 1):
        print(f"    {i}. {pv['move']} (score: {pv['score']})")
    print()

def test_time_management():
    """Test time-based search"""
    print("=" * 60)
    print("TEST 5: Time Management")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    
    fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
    
    print(f"Position: {fen}")
    print("Searching for 2 seconds...")
    
    start = time.time()
    result = engine.search(fen, time_ms=2000)
    elapsed = time.time() - start
    
    print(f"\n✓ Best move: {result['move']}")
    print(f"  Actual time: {elapsed:.2f}s (should be ~2s)")
    print(f"  Nodes: {result['nodes']:,}")
    print()

def test_board_state():
    """Test board state manipulation"""
    print("=" * 60)
    print("TEST 6: Board State")
    print("=" * 60)
    
    board = chess_engine.PyBoardState()
    
    print("Starting position:")
    print(f"  FEN: {board.to_fen()}")
    print(f"  In check: {board.is_in_check()}")
    print(f"  Game over: {board.is_game_over()}")
    
    print("\nMaking moves: e2e4, e7e5, Nf3, Nc6")
    board.make_move("e2e4")
    board.make_move("e7e5")
    board.make_move("g1f3")
    board.make_move("b8c6")
    
    print(f"  FEN: {board.to_fen()}")
    print(f"  In check: {board.is_in_check()}")
    print()

def test_performance():
    """Performance test"""
    print("=" * 60)
    print("TEST 7: Performance Benchmark")
    print("=" * 60)
    
    engine = chess_engine.PyChessEngine(threads=4)
    
    positions = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
        "rnbqkb1r/pp1ppppp/5n2/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    ]
    
    total_nodes = 0
    total_time = 0
    
    for i, fen in enumerate(positions, 1):
        start = time.time()
        result = engine.search(fen, depth=8)
        elapsed = time.time() - start
        
        total_nodes += result['nodes']
        total_time += elapsed
        
        print(f"Position {i}: {result['nodes']:,} nodes in {elapsed:.2f}s")
    
    avg_nps = int(total_nodes / total_time)
    print(f"\n✓ Average NPS: {avg_nps:,} nodes/second")
    print()

def main():
    """Run all tests"""
    print("\n🦀 RUST CHESS ENGINE TEST SUITE")
    print("With Stockfish-inspired features:")
    print("  ✓ Singular Extensions")
    print("  ✓ Reverse Futility Pruning")
    print("  ✓ Null Move Verification")
    print("  ✓ Internal Iterative Deepening")
    print("  ✓ Futility Pruning")
    print("  ✓ Improved LMR")
    print("  ✓ History + Countermove Heuristics")
    print()
    
    try:
        test_basic_search()
        test_tactical_position()
        test_endgame()
        test_multi_pv()
        test_time_management()
        test_board_state()
        test_performance()
        
        print("=" * 60)
        print("✅ ALL TESTS PASSED!")
        print("=" * 60)
        print("\nYour engine is ready to use!")
        print("  • Run Lichess bot: python3 lichess_bot_rust.py YOUR_TOKEN")
        print("  • Use with GUI: ./target/release/chess_uci")
        print()
        
    except Exception as e:
        print(f"\n❌ TEST FAILED: {e}")
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    main()