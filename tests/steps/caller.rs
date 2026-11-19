use cucumber::{given, then, when};
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

#[given(regex = r#"^a sandbox whose signing key is "([A-Z])"$"#)]
fn sandbox_signing_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
}

#[when(regex = r#"^the caller restores the dump under key "([A-Z])" into a new sandbox$"#)]
fn restore_dump_under_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
    restore_dump(world);
}

#[when("the caller restores the dump without its tag into a new sandbox")]
fn restore_dump_without_tag(world: &mut World) {
    let mut bytes = world.dump.clone().expect("a dump");
    bytes.truncate(bytes.len() - 32);
    world.restore(&bytes);
}

#[when(
    regex = r"^the caller restores the dump with its (first|middle|last) byte flipped into a new sandbox$"
)]
fn restore_dump_with_byte_flipped(world: &mut World, position: String) {
    let mut bytes = world.dump.clone().expect("a dump");
    let index = match position.as_str() {
        "first" => 0,
        "middle" => bytes.len() / 2,
        _ => bytes.len() - 1,
    };
    bytes[index] ^= 1;
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

#[then("the caller's error is that the signature doesn't match")]
fn error_is_signature_mismatch(world: &mut World) {
    assert_eq!(
        world.caller_error,
        Some(ghostmode::Error::SignatureMismatch)
    );
}
