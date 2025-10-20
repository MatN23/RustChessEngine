#!/usr/bin/env python3
"""Quick test to verify the engine is responding"""

import chess_engine
import time

print("Testing engine...")

# Create engine
engine = chess_engine.PyChessEngine(threads=4)
engine.set_hash_size(512)

# Test position
fen = "rnbqk2r/pp2ppbp/2p2np1/3p4/2P5/2NBPN2/PP1P1PPP/R1BQK2R w KQkq - 2 6"

print(f"Position: {fen}")
print("Searching with 5 second limit...")

start = time.time()
result = engine.search(fen, depth=20, time_ms=5000)
elapsed = time.time() - start

print(f"\nResult: {result}")
print(f"Elapsed: {elapsed:.2f}s")

if result.get('move'):
    print(f"✓ Engine is working! Best move: {result['move']}")
else:
    print("✗ Engine returned no move!")