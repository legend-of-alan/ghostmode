use cucumber::{then, when};
use ghostmode::DumpError;

use crate::{Request, World};

#[when(regex = r"^the caller cancels run (\d+)$")]
async fn cancel_run(world: &mut World, run_id: u64) {
    world.send(Request::Cancel(run_id)).await;
}

#[then(regex = r"^cancel returns (true|false)$")]
fn cancel_returns(world: &mut World, was_open: bool) {
    assert_eq!(world.cancel_returned, Some(was_open));
}

#[when("the caller resets the sandbox")]
async fn reset_sandbox(world: &mut World) {
    world.send(Request::Reset).await;
}

#[when("the caller dumps the sandbox")]
async fn dump_sandbox(world: &mut World) {
    world.send(Request::Dump).await;
}

#[when("the caller restores the dump into a new sandbox")]
fn restore_dump(world: &mut World) {
    let bytes = world.dump.clone().expect("a dump");
    world.restore(&bytes);
}

#[when(regex = r#"^the caller restores the bytes "(.*)" into a new sandbox$"#)]
fn restore_bytes(world: &mut World, text: String) {
    world.restore(text.as_bytes());
}

#[then(regex = r"^the caller's error is that run (\d+) is open$")]
fn error_is_run_open(world: &mut World, run_id: u64) {
    let expected = ghostmode::Error::RunOpen { run_id };
    assert_eq!(world.caller_error, Some(expected));
}

#[then("the caller's error is Monty's NotADump")]
fn error_is_not_a_dump(world: &mut World) {
    let expected = ghostmode::Error::Restore(DumpError::NotADump);
    assert_eq!(world.caller_error, Some(expected));
}
