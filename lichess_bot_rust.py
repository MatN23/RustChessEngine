"""
Lichess Bot with Rust Chess Engine Backend
Connects Rust engine to Lichess for blazing fast online play.
"""

import requests
import json
import time
import threading

# Import the Rust chess engine
try:
    import chess_engine
    RUST_ENGINE_AVAILABLE = True
    print("✓ Rust chess engine loaded successfully!")
except ImportError as e:
    print(f"✗ Failed to load Rust engine: {e}")
    print("Please build the Rust engine with: maturin develop --release")
    RUST_ENGINE_AVAILABLE = False
    exit(1)


class LichessBot:
    """Bridge between Lichess API and Rust chess engine."""
    
    def __init__(self, token: str):
        self.token = token
        self.base_url = "https://lichess.org"
        self.headers = {"Authorization": f"Bearer {token}"}
        
        # Initialize Rust engine
        self.engine = chess_engine.PyChessEngine(threads=4)
        
        self.active_games = {}
        self.move_overhead = 500
        
    def start(self):
        """Start the bot - accept challenges and play games."""
        print("Starting Lichess bot with RUST ENGINE...")
        print(f"Profile: {self.base_url}/@/{self.get_account()['username']}")
        
        event_thread = threading.Thread(target=self.handle_events, daemon=True)
        event_thread.start()
        
        print("\n🚀 Bot is running with RUST POWER! Waiting for challenges...")
        print("Press Ctrl+C to stop\n")
        
        try:
            event_thread.join()
        except KeyboardInterrupt:
            print("\nStopping bot...")
    
    def get_account(self):
        """Get bot account info."""
        response = requests.get(
            f"{self.base_url}/api/account",
            headers=self.headers
        )
        return response.json()
    
    def handle_events(self):
        """Listen for challenges and game starts."""
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
        """Process incoming events."""
        event_type = event.get('type')
        
        if event_type == 'challenge':
            self.handle_challenge(event['challenge'])
        
        elif event_type == 'gameStart':
            game_id = event['game']['id']
            print(f"\n=== Game started: {game_id} ===")
            game_thread = threading.Thread(
                target=self.play_game,
                args=(game_id,),
                daemon=True
            )
            game_thread.start()
    
    def handle_challenge(self, challenge):
        """Accept or decline challenges."""
        challenge_id = challenge['id']
        challenger = challenge['challenger']['name']
        variant = challenge.get('variant', {}).get('key', 'standard')
        
        print(f"\nChallenge from {challenger}")
        print(f"  Variant: {variant}")
        
        if variant == 'standard':
            self.accept_challenge(challenge_id)
            print(f"  ✓ Accepted!")
        else:
            self.decline_challenge(challenge_id)
            print(f"  ✗ Declined (only standard chess)")
    
    def accept_challenge(self, challenge_id):
        """Accept a challenge."""
        url = f"{self.base_url}/api/challenge/{challenge_id}/accept"
        requests.post(url, headers=self.headers)
    
    def decline_challenge(self, challenge_id):
        """Decline a challenge."""
        url = f"{self.base_url}/api/challenge/{challenge_id}/decline"
        requests.post(url, headers=self.headers)
    
    def play_game(self, game_id):
        """Play a game using the Rust engine."""
        url = f"{self.base_url}/api/bot/game/stream/{game_id}"
        current_fen = None
        my_color = None
        move_times = []
        
        print(f"Playing game: {self.base_url}/{game_id}")
        
        try:
            with requests.get(url, headers=self.headers, stream=True, timeout=30) as response:
                if response.status_code != 200:
                    print(f"Error connecting to game: {response.status_code}")
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
                                
                                print(f"Playing as {my_color.upper()}")
                                print(f"White: {event.get('white', {}).get('name', 'Unknown')}")
                                print(f"Black: {event.get('black', {}).get('name', 'Unknown')}")
                                
                                if my_color == 'white' and event['state']['moves'] == '':
                                    self.make_move(game_id, current_fen, event['state'], my_color, move_times)
                            
                            elif event_type == 'gameState':
                                current_fen = self.update_position(current_fen, event)
                                self.make_move(game_id, current_fen, event, my_color, move_times)
                            
                            elif event_type == 'chatLine':
                                username = event.get('username', 'unknown')
                                text = event.get('text', '')
                                print(f"💬 Chat from {username}: {text}")
                            
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
            print(f"Unexpected error in game {game_id}: {e}")
        
        print(f"Game {game_id} finished\n")
    
    def setup_game(self, game_full):
        """Setup board from game start."""
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
        """Update position with new moves."""
        moves_str = state.get('moves', '')
        
        if not moves_str:
            return "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        
        board = chess_engine.PyBoardState()
        for move_str in moves_str.split():
            board.make_move(move_str)
        
        return board.to_fen()
    
    def make_move(self, game_id, fen, state, my_color, move_times):
        """Calculate and make a move using Rust engine."""
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
        
        if wtime > 1000000000:
            wtime = 60000
        if btime > 1000000000:
            btime = 60000
        
        my_time = wtime if my_color == 'white' else btime
        my_inc = winc if my_color == 'white' else binc
        
        time_for_move = self.calculate_time_allocation(
            my_time, my_inc, move_count, move_times
        )
        
        time_for_move = max(100, time_for_move - self.move_overhead)
        
        print(f"\n{'='*50}")
        print(f"Move #{move_count + 1} - My turn ({my_color})")
        print(f"Time: {my_time}ms remaining (+{my_inc}ms inc)")
        print(f"Thinking for: {int(time_for_move)}ms")
        print(f"{'='*50}")
        
        start_time = time.time()
        
        # Call Rust engine!
        result = self.engine.search(
            fen=fen,
            depth=64,
            time_ms=int(time_for_move)
        )
        
        elapsed_time = (time.time() - start_time) * 1000
        move_times.append(elapsed_time)
        
        best_move = result.get('move')
        score = result.get('score', 0)
        nodes = result.get('nodes', 0)
        
        if best_move:
            print(f"\n▶ Playing: {best_move}")
            print(f"  Score: {score}cp")
            print(f"  Nodes: {nodes:,}")
            print(f"  Time used: {int(elapsed_time)}ms")
            print(f"  NPS: {int(nodes / (elapsed_time / 1000)) if elapsed_time > 0 else 0:,}")
            print(f"{'='*50}\n")
            
            self.send_move(game_id, best_move)
        else:
            print("No legal moves (game over)")
            print(f"{'='*50}\n")
    
    def calculate_time_allocation(self, my_time, my_inc, move_count, move_times):
        """Calculate time allocation for this move."""
        moves_remaining = max(20, 40 - move_count // 2)
        base_time = (my_time / moves_remaining) + (my_inc * 0.5)
        
        multiplier = 1.0
        
        if move_count < 10:
            multiplier = 1.2
        elif move_count < 30:
            multiplier = 2.0
        else:
            multiplier = 3.0
        
        if my_time < 30000:
            multiplier *= 0.7
        if my_time < 10000:
            multiplier *= 0.5
        
        time_for_move = base_time * multiplier
        
        time_for_move = max(1500, min(time_for_move, my_time * 0.25))
        time_for_move = min(time_for_move, 90000)
        
        min_cushion = max(3000, my_time * 0.1)
        time_for_move = min(time_for_move, my_time - min_cushion)
        
        return time_for_move
    
    def send_move(self, game_id, move_uci):
        """Send move to Lichess."""
        url = f"{self.base_url}/api/bot/game/{game_id}/move/{move_uci}"
        response = requests.post(url, headers=self.headers)
        
        if response.status_code != 200:
            print(f"Error sending move: {response.text}")


def main():
    """Main entry point."""
    import sys
    import os
    
    token = os.environ.get('LICHESS_TOKEN')
    
    if not token and len(sys.argv) > 1:
        token = sys.argv[1]
    
    if not token:
        print("Usage: python lichess_bot_rust.py YOUR_LICHESS_TOKEN")
        print("Or set LICHESS_TOKEN environment variable")
        sys.exit(1)
    
    bot = LichessBot(token)
    bot.start()


if __name__ == "__main__":
    main()