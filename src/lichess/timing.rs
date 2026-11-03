//! export FIXED_MOVE_TIME_MS=5000 to override the default 5s per move allocation.

//! Per-move time allocation. The core formula is a 1:1 port of
//! `calculate_time_allocation` from the old Python bot; `move_time_ms` adds a
//! final safety cap that the Python version lacked.

/// Raw allocation in ms (before overhead is subtracted).
/// `move_count` is the number of half-moves played so far, exactly as the
/// Python bot used `len(moves.split())`.
pub fn calculate_time_allocation(my_time: u64, my_inc: u64, move_count: usize) -> f64 {
    let my_time_f = my_time as f64;

    // Base split: assume ~30 moves left.
    let moves_remaining = std::cmp::max(10, 30i64 - move_count as i64) as f64;
    let base_time = my_time_f / moves_remaining + my_inc as f64 * 0.5;

    let phase_multiplier = if move_count < 10 {
        0.6
    } else if move_count < 25 {
        0.9
    } else {
        1.0
    };

    let time_for_move = base_time * phase_multiplier;

    let (min_time, max_time) = if my_time > 600_000 {
        (1000.0, 10_000.0)
    } else if my_time > 180_000 {
        (500.0, 5000.0)
    } else {
        (200.0, 2000.0)
    };

    time_for_move.clamp(min_time, max_time)
}

/// Final search budget in ms.
///
/// Python did `max(200, alloc - overhead)`, which can exceed the remaining clock
/// when very low on time (e.g. 400 ms left -> 200 ms search + 300 ms lag = flag).
/// We additionally never plan to spend more than half of the time left after
/// overhead.
pub fn move_time_ms(my_time: u64, my_inc: u64, move_count: usize, overhead_ms: u64) -> u64 {
    // A fixed override can request a target budget, but it must never
    // bypass the clock-safety cap. Without an override, adapt to the clock,
    // increment, and move number as the original allocator intended.
    let allocation_ms = std::env::var("FIXED_MOVE_TIME_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v as f64)
        .unwrap_or_else(|| calculate_time_allocation(my_time, my_inc, move_count));

    let wanted = (allocation_ms - overhead_ms as f64).max(200.0) as u64;
    let clock_cap = (my_time.saturating_sub(overhead_ms) / 2).max(50);
    wanted.min(clock_cap)
}
