use crate::ui::state::DebounceState;
use std::time::{Duration, Instant};

pub fn should_send_debounced(
    now: Instant,
    current_value: f32,
    debounce: &mut DebounceState,
    min_delta: f32,
    min_interval: Duration,
) -> Option<f32> {
    let rounded = current_value.round();

    let changed_enough = (rounded - debounce.last_sent_value).abs() >= min_delta;
    let interval_elapsed = now.duration_since(debounce.last_send_time) >= min_interval;

    if changed_enough && interval_elapsed {
        debounce.last_sent_value = rounded;
        debounce.last_send_time = now;
        Some(rounded)
    } else {
        None
    }
}
