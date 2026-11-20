use std::time::Duration;

use cucumber::{given, then, when};
use futures_timer::Delay;
use ghostmode::Limits;

use crate::World;

#[given(regex = r"^a sandbox whose execute wait is (\d+) ms and poll wait is (\d+) ms$")]
fn sandbox_waits(world: &mut World, execute_ms: u64, poll_ms: u64) {
    world.execute_wait = Some(Duration::from_millis(execute_ms));
    world.poll_wait = Some(Duration::from_millis(poll_ms));
}

#[given(regex = r"^a sandbox whose `(\w+)` is (\d+)( ms| MiB|)$")]
fn sandbox_limit(world: &mut World, field: String, number: usize, unit: String) {
    let limits = world.limits.get_or_insert_with(Limits::default);
    let millis = Duration::from_millis(number as u64);
    match (field.as_str(), unit.as_str()) {
        ("run_time", " ms") => limits.run_time = millis,
        ("memory_bytes", " MiB") => limits.memory_bytes = number * 1024 * 1024,
        ("calls_per_execute", "") => limits.calls_per_execute = number,
        ("call_timeout", " ms") => limits.call_timeout = Some(millis),
        ("idle_timeout", " ms") => limits.idle_timeout = Some(millis),
        ("output_chars", "") => limits.output_chars = number,
        _ => panic!("no limit `{field}` with unit `{}`", unit.trim()),
    }
}

#[when(regex = r"^(\d+) ms pass$")]
async fn time_passes(_world: &mut World, ms: u64) {
    Delay::new(Duration::from_millis(ms)).await;
}

#[then(
    "the default limits are 30 s of run time, 256 MiB, a recursion depth of 1000, 100 calls per \
     execute, no call or idle timeout and 40,000 output characters"
)]
fn defaults_are(_world: &mut World) {
    let limits = Limits::default();
    assert_eq!(limits.run_time, Duration::from_secs(30));
    assert_eq!(limits.memory_bytes, 256 * 1024 * 1024);
    assert_eq!(limits.recursion_depth, 1000);
    assert_eq!(limits.calls_per_execute, 100);
    assert_eq!(limits.call_timeout, None);
    assert_eq!(limits.idle_timeout, None);
    assert_eq!(limits.output_chars, 40_000);
}
