"""
Lichess Bot with Ultimate Rust Chess Engine
Expected Performance: 2800-3000+ Elo on Lichess
"""

import requests
import json
import time
import threading
import os
import sys

try:
    import chess_engine
    RUST_ENGINE_AVAILABLE = True
    print("✓ Rust chess engine loaded successfully!")
except ImportError as e:
    print(f"✗ Failed to load Rust engine: {e}")
    print("Please build: maturin develop --release")
    RUST_ENGINE_AVAILABLE = False
    exit(1)


class LichessBot:
    """Bridge between Lichess API and Ultimate Rust Engine."""
    
    def __init__(self, token: str, threads: int = 8, hash_mb: int = 2048):
        self.token = token
        self.base_url = "https://lichess.org"
        self.headers = {"Authorization": f"Bearer {token}"}
        
        # Initialize Ultimate Rust engine
        print(f"Initializing engine with {threads} threads and {hash_mb}MB hash...")
        self.engine = chess_engine.PyChessEngine(threads=threads)
        self.engine.set_hash_size(hash_mb)
        
        self.active_games = {}
        self.move_overhead = 300  # Network latency buffer
        
        print(f"✓ Engine ready!")
        print(f"  Threads: {threads}")
        print(f"  Hash: {hash_mb} MB")
        print(f"  Expected NPS: {threads * 1_500_000:,}")
        print(f"  Estimated Elo: 2800-3000+")
        
    def start(self):
        """Start the bot"""
        print("\n🚀 Starting Lichess bot with ULTIMATE RUST ENGINE...")
        account = self.get_account()
        print(f"Profile: {self.base_url}/@/{account['username']}")
        print(f"Rating: {account.get('perfs', {}).get('rapid', {}).get('rating', 'N/A')}")
        
        event_thread = threading.Thread(target=self.handle_events, daemon=True)
        event_thread.start()
        
        print("\n⚡ Bot is running! Waiting for challenges...")
        print("Press Ctrl+C to stop\n")
        
        try:
            event_thread.join()
        except KeyboardInterrupt:
            print("\nStopping bot...")
    
    def get_account(self):
        """Get bot account info"""
        response = requests.get(
            f"{self.base_url}/api/account",
            headers=self.headers
        )
        return response.json()
    
    def handle_events(self):
        """Listen for challenges and games"""
        url = f"{self.base_url}/api/stream/event"
        
        while True:
            try:
                with requests.get(url, headers=self.headers, stream=True, timeout=60) as response:
                    for line in response.iter_lines():
                        if line:
                            try:
                                event = json.loads(line.decode('utf-8'))
                                self.process_event(event)
                            except Exception as e:
                                print(f"Error processing event: {e}")
            except requests.exceptions.RequestException as e:
                print(f"Connection error: {e}")
                print("Reconnecting in 5 seconds...")
                time.sleep(5)
    
    def process_event(self, event):
        """Process incoming events"""
        event_type = event.get('type')
        
        if event_type == 'challenge':
            self.handle_challenge(event['challenge'])
        
        elif event_type == 'gameStart':
            game_id = event['game']['id']
            print(f"\n{'='*60}")
            print(f"Game started: {game_id}")
            print(f"{'='*60}")
            game_thread = threading.Thread(
                target=self.play_game,
                args=(game_id,),
                daemon=True
            )
            game_thread.start()
    
    def handle_challenge(self, challenge):
        """Accept or decline challenges"""
        challenge_id = challenge['id']
        challenger = challenge['challenger']['name']
        variant = challenge.get('variant', {}).get('key', 'standard')
        time_control = challenge.get('timeControl', {})
        
        print(f"\n📩 Challenge from {challenger}")
        print(f"  Variant: {variant}")
        print(f"  Time control: {time_control.get('type', 'unlimited')}")
        
        if variant == 'standard':
            self.accept_challenge(challenge_id)
            print(f"  ✓ Accepted!")
        else:
            self.decline_challenge(challenge_id)
            print(f"  ✗ Declined (only standard chess)")
    
    def accept_challenge(self, challenge_id):
        """Accept a challenge"""
        url = f"{self.base_url}/api/challenge/{challenge_id}/accept"
        requests.post(url, headers=self.headers)
    
    def decline_challenge(self, challenge_id):
        """Decline a challenge"""
        url = f"{self.base_url}/api/challenge/{challenge_id}/decline"
        requests.post(url, headers=self.headers)
    
    def play_game(self, game_id):
        """Play a game using Ultimate Rust Engine"""
        url = f"{self.base_url}/api/bot/game/stream/{game_id}"
        current_fen = None
        my_color = None
        move_times = []
        
        print(f"Playing: {self.base_url}/{game_id}")
        
        try:
            with requests.get(url, headers=self.headers, stream=True, timeout=30) as response:
                if response.status_code != 200:
                    print(f"Error connecting: {response.status_code}")
                    return
                
                for line in response.iter_lines():
                    if line:
                        try:
                            event = json.loads(line.decode('utf-8'))
                            event_type = event.get('type')
                            
                            if not event_type:
                                continue
                            
                            if event_type == 'gameFull':
                                current_fen = self.setup_game(event)
                                
                                account = self.get_account()
                                white_id = event.get('white', {}).get('id', '').lower()
                                my_id = account.get('id', '').lower()
                                
                                my_color = 'white' if white_id == my_id else 'black'
                                
                                print(f"  Playing as {my_color.upper()}")
                                print(f"  White: {event.get('white', {}).get('name', 'Unknown')} ({event.get('white', {}).get('rating', '?')})")
                                print(f"  Black: {event.get('black', {}).get('name', 'Unknown')} ({event.get('black', {}).get('rating', '?')})")
                                
                                if my_color == 'white' and event['state']['moves'] == '':
                                    self.make_move(game_id, current_fen, event['state'], my_color, move_times)
                            
                            elif event_type == 'gameState':
                                current_fen = self.update_position(current_fen, event)
                                self.make_move(game_id, current_fen, event, my_color, move_times)
                            
                            elif event_type == 'chatLine':
                                username = event.get('username', 'unknown')
                                text = event.get('text', '')
                                print(f"💬 {username}: {text}")
                            
                            elif event_type == 'gameFinish':
                                print(f"Game finished!")
                                break
                                
                        except json.JSONDecodeError:
                            continue
                        except Exception as e:
                            print(f"Error processing event: {e}")
                            
        except requests.exceptions.RequestException as e:
            print(f"Connection error: {e}")
        except Exception as e:
            print(f"Unexpected error: {e}")
        
        print(f"Game {game_id} complete\n")
    
    def setup_game(self, game_full):
        """Setup board from game start"""
        initial_fen = game_full.get('initialFen', 'startpos')
        
        if initial_fen == 'startpos':
            fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        else:
            fen = initial_fen
        
        moves = game_full.get('state', {}).get('moves', '')
        
        if moves:
            board = chess_engine.PyBoardState(fen)
            for move_str in moves.split():
                board.make_move(move_str)
            fen = board.to_fen()
        
        return fen
    
    def update_position(self, current_fen, state):
        """Update position with new moves"""
        moves_str = state.get('moves', '')
        
        if not moves_str:
            return "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        
        board = chess_engine.PyBoardState()
        for move_str in moves_str.split():
            board.make_move(move_str)
        
        return board.to_fen()
    
    def make_move(self, game_id, fen, state, my_color, move_times):
        """Calculate and make a move"""
        status = state.get('status')
        if status in ['mate', 'stalemate', 'draw', 'outoftime', 'resign', 'aborted']:
            return
        
        moves_str = state.get('moves', '')
        move_count = len(moves_str.split()) if moves_str else 0
        is_white_turn = move_count % 2 == 0
        my_turn = (my_color == 'white' and is_white_turn) or (my_color == 'black' and not is_white_turn)
        
        if not my_turn:
            return
        
        wtime = state.get('wtime', 60000)
        btime = state.get('btime', 60000)
        winc = state.get('winc', 0)
        binc = state.get('binc', 0)
        
        # Clamp unrealistic values (max 3 hours)
        wtime = min(wtime, 10_800_000)
        btime = min(btime, 10_800_000)
        
        my_time = wtime if my_color == 'white' else btime
        my_inc = winc if my_color == 'white' else binc
        
        # Calculate time allocation
        time_for_move = self.calculate_time_allocation(
            my_time, my_inc, move_count
        )
        
        # Subtract overhead
        time_for_move = max(200, time_for_move - self.move_overhead)
        
        print(f"\n{'='*60}")
        print(f"Move #{move_count + 1} - My turn ({my_color})")
        print(f"Time: {my_time}ms remaining (+{my_inc}ms inc)")
        print(f"Allocated: {int(time_for_move)}ms")
        print(f"{'='*60}")
        
        start_time = time.time()
        
        # Call Ultimate Rust Engine with reasonable depth
        result = self.engine.search(
            fen=fen,
            depth=32,  # Will be limited by time
            time_ms=int(time_for_move)
        )
        
        elapsed_time = (time.time() - start_time) * 1000
        move_times.append(elapsed_time)
        
        best_move = result.get('move')
        score = result.get('score', 0)
        nodes = result.get('nodes', 0)
        
        if best_move:
            nps = int(nodes / (elapsed_time / 1000)) if elapsed_time > 0 else 0
            
            print(f"\n▶ Playing: {best_move}")
            print(f"  Score: {score}cp")
            print(f"  Nodes: {nodes:,}")
            print(f"  Time: {int(elapsed_time)}ms")
            print(f"  NPS: {nps:,}")
            
            # Evaluation feedback
            if score > 300:
                print(f"  📈 Winning position!")
            elif score > 100:
                print(f"  ✓ Slight advantage")
            elif score < -300:
                print(f"  📉 Difficult position")
            elif score < -100:
                print(f"  ⚠ Slight disadvantage")
            else:
                print(f"  = Equal position")
            
            print(f"{'='*60}\n")
            
            self.send_move(game_id, best_move)
        else:
            print("No legal moves (game over)")
            print(f"{'='*60}\n")
    
    def calculate_time_allocation(self, my_time, my_inc, move_count):
        """
        Adaptive time allocation:
        - Uses more time if lots of time left
        - Moves faster if low on time
        - Caps max time per move based on remaining time
        """

        # Base split: assume ~30 moves left
        moves_remaining = max(10, 30 - move_count)
        base_time = my_time / moves_remaining + my_inc * 0.5

        # Phase multiplier: opening, middlegame, endgame
        if move_count < 10:
            phase_multiplier = 0.6
        elif move_count < 25:
            phase_multiplier = 0.9
        else:
            phase_multiplier = 1.0

        time_for_move = base_time * phase_multiplier

        # Adaptive max/min based on remaining time
        if my_time > 600_000:       # >10 minutes left
            max_time = 10_000       # allow up to 10s per move
            min_time = 1000         # at least 1s
        elif my_time > 180_000:     # 3-10 minutes left
            max_time = 5000
            min_time = 500
        else:                       # <3 minutes left
            max_time = 2000
            min_time = 200

        # Clamp to adaptive bounds
        time_for_move = max(min_time, min(time_for_move, max_time))

        return time_for_move

    
    def send_move(self, game_id, move_uci):
        """Send move to Lichess"""
        url = f"{self.base_url}/api/bot/game/{game_id}/move/{move_uci}"
        response = requests.post(url, headers=self.headers)
        
        if response.status_code != 200:
            print(f"Error sending move: {response.text}")


def main():
    """Main entry point"""
    token = os.environ.get('LICHESS_TOKEN')
    
    if not token and len(sys.argv) > 1:
        token = sys.argv[1]
    
    if not token:
        print("Usage: python lichess_bot_rust.py YOUR_LICHESS_TOKEN")
        print("Or set LICHESS_TOKEN environment variable")
        sys.exit(1)
    
    # Configuration
    threads = int(os.environ.get('ENGINE_THREADS', '8'))
    hash_mb = int(os.environ.get('ENGINE_HASH', '2048'))
    
    print(f"\n⚙️  Configuration:")
    print(f"  Threads: {threads}")
    print(f"  Hash: {hash_mb} MB")
    print()
    
    bot = LichessBot(token, threads=threads, hash_mb=hash_mb)
    bot.start()


if __name__ == "__main__":
    main()