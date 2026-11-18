use std::time::Duration;

use cucumber::{then, when};
use futures_timer::Delay;
use ghostmode::Limits;

use crate::World;

#[when(regex = r"^(\d+) ms pass$")]
async fn time_passes(_world: &mut World, ms: u64) {
    Delay::new(Duration::from_millis(ms)).await;
}

#[then(
    "the default limits are 30 s of run time, 256 MiB, a recursion depth of 1000, 100 calls per \
     execute and no call or idle timeout"
)]
fn defaults_are(_world: &mut World) {
    let expected = Limits {
        run_time: Duration::from_secs(30),
        memory_bytes: 256 * 1024 * 1024,
        recursion_depth: 1000,
        calls_per_execute: 100,
        call_timeout: None,
        idle_timeout: None,
    };
    assert_eq!(Limits::default(), expected);
}
