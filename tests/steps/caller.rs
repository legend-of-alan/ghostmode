use cucumber::{given, then, when};
use futures::FutureExt;

use crate::{Request, World};

#[when(regex = r"^the caller cancels run (\d+)$")]
async fn cancel_run(world: &mut World, run_id: u64) {
    world.send(Request::Cancel(run_id)).await;
}

#[then(regex = r"^cancel returns (true|false)$")]
fn cancel_returns(world: &mut World, was_open: bool) {
    assert_eq!(world.cancel_returned, Some(was_open));
}

#[when(regex = r"^the caller stops awaiting the model's execute of (.+)$")]
async fn drop_execute(world: &mut World, code: String) {
    let mut sandbox = world.take_sandbox().await;
    let request = Request::Execute {
        code,
        restart: false,
    };
    let reply = sandbox.reply(request).now_or_never();
    assert!(
        reply.is_none(),
        "execute answered before the caller stopped awaiting it"
    );
    world.sandbox = Some(sandbox);
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
async fn restore_dump(world: &mut World) {
    let bytes = world.dump.clone().expect("a dump");
    world.restore(&bytes).await;
}

#[given(regex = r#"^a sandbox whose signing key is "([A-Z])"$"#)]
fn sandbox_signing_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
}

#[when(regex = r#"^the caller restores the dump under key "([A-Z])" into a new sandbox$"#)]
async fn restore_dump_under_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
    restore_dump(world).await;
}

#[when("the caller restores the dump without its tag into a new sandbox")]
async fn restore_dump_without_tag(world: &mut World) {
    let mut bytes = world.dump.clone().expect("a dump");
    bytes.truncate(bytes.len() - 32);
    world.restore(&bytes).await;
}

#[when(
    regex = r"^the caller restores the dump with its (first|middle|last) byte flipped into a new sandbox$"
)]
async fn restore_dump_with_byte_flipped(world: &mut World, position: String) {
    let mut bytes = world.dump.clone().expect("a dump");
    let index = match position.as_str() {
        "first" => 0,
        "middle" => bytes.len() / 2,
        _ => bytes.len() - 1,
    };
    bytes[index] ^= 1;
    world.restore(&bytes).await;
}

#[when(regex = r#"^the caller restores the bytes "(.*)" into a new sandbox$"#)]
async fn restore_bytes(world: &mut World, text: String) {
    world.restore(text.as_bytes()).await;
}

#[then(regex = r"^the caller's error is that run (\d+) is open$")]
fn error_is_run_open(world: &mut World, run_id: u64) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(
        matches!(error, ghostmode::Error::RunOpen { run_id: open } if *open == run_id),
        "{error:?}"
    );
}

#[then("the caller's error is that the backend can't restore the bytes")]
fn error_is_restore(world: &mut World) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(matches!(error, ghostmode::Error::Restore(_)), "{error:?}");
}

#[then("the caller's error is that the signature doesn't match")]
fn error_is_signature_mismatch(world: &mut World) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(
        matches!(error, ghostmode::Error::SignatureMismatch),
        "{error:?}"
    );
}
