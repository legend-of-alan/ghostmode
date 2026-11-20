use cucumber::{given, then, when};
use futures::FutureExt;

use crate::{Request, World};

/// How many bytes the header takes that starts an unsigned dump: the format version, the
/// `restarted` byte and the 8-byte last run id.
const DUMP_HEADER_BYTES: usize = 10;

#[given(regex = r#"^a sandbox whose signing key is "([A-Z])"$"#)]
fn sandbox_signing_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
}

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

#[when("the caller stops awaiting the dump")]
async fn drop_dump(world: &mut World) {
    let mut sandbox = world.take_sandbox().await;
    let reply = sandbox.reply(Request::Dump).now_or_never();
    assert!(
        reply.is_none(),
        "dump answered before the caller stopped awaiting it"
    );
    world.sandbox = Some(sandbox);
}

#[when("the caller resets the sandbox")]
async fn reset_sandbox(world: &mut World) {
    world.send(Request::Reset).await;
}

#[when("the caller drops the sandbox")]
fn drop_sandbox(world: &mut World) {
    world.sandbox.take().expect("a sandbox");
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

#[when(regex = r#"^the caller restores the dump under key "([A-Z])" into a new sandbox$"#)]
async fn restore_dump_under_key(world: &mut World, letter: char) {
    world.signing_key = Some([letter as u8; 32]);
    restore_dump(world).await;
}

#[when(
    regex = r"^the caller restores the dump with its (first|last) byte flipped into a new sandbox$"
)]
async fn restore_dump_with_byte_flipped(world: &mut World, position: String) {
    let mut bytes = world.dump.clone().expect("a dump");
    let index = match position.as_str() {
        "first" => 0,
        _ => bytes.len() - 1,
    };
    bytes[index] ^= 1;
    world.restore(&bytes).await;
}

#[when("the caller restores the dump cut to its header into a new sandbox")]
async fn restore_dump_cut_to_header(world: &mut World) {
    let mut bytes = world.dump.clone().expect("a dump");
    bytes.truncate(DUMP_HEADER_BYTES);
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

#[then("the caller's error is that the bytes don't restore")]
fn error_is_restore(world: &mut World) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(matches!(error, ghostmode::Error::Restore(_)), "{error:?}");
}

#[then(regex = r"^the caller's error is that the dump is format version (\d+), not version (\d+)$")]
fn error_is_other_format_version(world: &mut World, found: u8, expected: u8) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(matches!(error, ghostmode::Error::Restore(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains(&format!("format version {found}, not version {expected}")),
        "{message}"
    );
}

#[then("the caller's error is that the signature doesn't match")]
fn error_is_signature_mismatch(world: &mut World) {
    let error = world.caller_error.as_ref().expect("a caller error");
    assert!(
        matches!(error, ghostmode::Error::SignatureMismatch),
        "{error:?}"
    );
}

#[then("the sandbox's Debug output doesn't show the signing key")]
fn debug_hides_signing_key(world: &mut World) {
    let key = world.signing_key.expect("a signing key");
    let sandbox = world.sandbox.as_ref().expect("a sandbox");
    let output = format!("{sandbox:?}");
    assert!(output.starts_with("Sandbox"), "{output}");
    assert!(!output.contains(&format!("{key:?}")), "{output}");
}
