import chess, chess.engine, chess.pgn, sys, math, json, datetime
E="/Users/matias/Documents/Projects/AI/RustChessEngine/target/release/chess_uci"; SF="/opt/homebrew/bin/stockfish"
elo=int(sys.argv[1]); movetime=float(sys.argv[2]); npairs=int(sys.argv[3]); out=sys.argv[4]
OPEN=["e2e4 e7e5 g1f3 b8c6","d2d4 d7d5 c2c4 e7e6","e2e4 c7c5 g1f3 d7d6","d2d4 g8f6 c2c4 g7g6","e2e4 e7e6 d2d4 d7d5","c2c4 e7e5 b1c3 g8f6","g1f3 d7d5 g2g3 g8f6","e2e4 c7c6 d2d4 d7d5"]
eng=chess.engine.SimpleEngine.popen_uci(E); eng.configure({"Threads":1,"Hash":64})
sf=chess.engine.SimpleEngine.popen_uci(SF); sf.configure({"Threads":1,"Hash":64,"UCI_LimitStrength":True,"UCI_Elo":elo})
W=D=L=0; fails=[]; pgn=open(out+".pgn","w")
for i in range(npairs):
  op=OPEN[i%len(OPEN)].split()
  for white_is_eng in (True,False):
    b=chess.Board()
    for m in op: b.push_uci(m)
    g=chess.pgn.Game(); g.headers["White"]="Rust" if white_is_eng else f"SF{elo}"; g.headers["Black"]=f"SF{elo}" if white_is_eng else "Rust"
    node=g
    for m in b.move_stack: node=node.add_variation(m)
    res=None
    while not b.is_game_over(claim_draw=True) and len(b.move_stack)<300:
      e = eng if (b.turn==chess.WHITE)==white_is_eng else sf
      try: r=e.play(b,chess.engine.Limit(time=movetime))
      except Exception as ex: fails.append((i,str(ex)[:80])); res="crash"; break
      if r.move not in b.legal_moves: fails.append((i,"illegal "+str(r.move))); res="illegal"; break
      b.push(r.move); node=node.add_variation(r.move)
    if res in ("crash","illegal"): 
      score=0.0 if (e is eng) else 1.0
    else:
      o=b.outcome(claim_draw=True); 
      if o is None or o.winner is None: score=0.5
      else: score=1.0 if (o.winner==chess.WHITE)==white_is_eng else 0.0
    g.headers["Result"]={1.0:"1-0",0.0:"0-1",0.5:"1/2-1/2"}[score] if white_is_eng else {1.0:"0-1",0.0:"1-0",0.5:"1/2-1/2"}[score]
    print(g,file=pgn,end="\n\n"); pgn.flush()
    W+=score==1; L+=score==0; D+=score==0.5
    print(f"game {2*i+1+(not white_is_eng)}: eng_score={score} plies={len(b.move_stack)} W{W} D{D} L{L}",flush=True)
    if res in ("crash",):
      try: eng.quit()
      except: pass
      eng=chess.engine.SimpleEngine.popen_uci(E); eng.configure({"Threads":1,"Hash":64})
n=W+D+L; s=(W+0.5*D)/n
def elo_of(p): return -400*math.log10(1/p-1) if 0<p<1 else float('inf')*(1 if p>=1 else -1)
var=(W*(1-s)**2+D*(0.5-s)**2+L*(0-s)**2)/n; sd=math.sqrt(var/n)
lo,hi=s-1.96*sd,s+1.96*sd
res=dict(sf_elo=elo,movetime=movetime,games=n,W=W,D=D,L=L,score=s,elo_diff=elo_of(s),ci95=[elo_of(max(lo,1e-6)),elo_of(min(hi,1-1e-6))],failures=fails,date=str(datetime.date.today()))
json.dump(res,open(out+".json","w"),indent=1); print(json.dumps(res)); eng.quit(); sf.quit()
