import subprocess, chess, sys
POS = [
 ("startpos","rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",4),
 ("kiwipete","r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",3),
 ("pos3","8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",4),
 ("pos4","r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",3),
 ("pos4m","r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1",3),
 ("pos5","rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",3),
 ("pos6","r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",3),
 ("ep-pin","8/8/8/8/k2Pp2Q/8/8/3K4 b - d3 0 1",3),
 ("promo","n1n5/PPPk4/8/8/8/8/4Kppp/5N1N b - - 0 1",3),
 ("castle-thru-check","r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",3),
]
def ref(b,d):
    if d==0: return 1
    return sum(ref_push(b,m,d) for m in list(b.legal_moves))
def ref_push(b,m,d):
    b.push(m); n=ref(b,d-1); b.pop(); return n
bad=0
for name,fen,d in POS:
    b=chess.Board(fen); want=ref(b,d)
    out=subprocess.run(["/home/claude/build/release/chess_uci"],input=f"position fen {fen}\nperft {d}\nquit\n",capture_output=True,text=True,timeout=300).stdout
    got=[l for l in out.splitlines() if l.startswith("Nodes searched")]
    got=int(got[0].split()[-1]) if got else None
    ok = got==want; bad+= not ok
    print(f"{'OK ' if ok else 'BAD'} {name:18} d{d} engine={got} ref={want}")
sys.exit(bad)
