use std::num::NonZero;
use std::process::{self, Command};
use std::thread;

use cucumber::when;

use crate::{Reply, Request, World};

/// Kills the `monty` worker running the session's script: the `monty` process whose parent is
/// this test process. The test pool keeps no spare worker, so it is the only one the test starts.
/// Fails when the test process has no `monty` worker.
#[when("the session's monty worker is killed")]
fn kill_session_worker(_world: &mut World) {
    let parent_id = process::id().to_string();
    let mut kill = if cfg!(windows) {
        let mut powershell = Command::new("powershell");
        let filter = format!("Name='monty.exe' AND ParentProcessId={parent_id}");
        let script = format!(
            "$workers = Get-CimInstance Win32_Process -Filter \"{filter}\"; \
             if (-not $workers) {{ exit 1 }}; Stop-Process -Id $workers.ProcessId -Force"
        );
        powershell.args(["-NoProfile", "-Command", &script]);
        powershell
    } else {
        let mut pkill = Command::new("pkill");
        pkill.args(["-KILL", "-P", &parent_id, "-x", "monty"]);
        pkill
    };
    let status = kill.status().expect("the process killer runs");
    assert!(status.success(), "{status}");
}

/// Opens one sandbox per worker the pool allows, which is the machine's available parallelism,
/// and runs a script in each so that each holds its worker. Fails if any of them can't run it.
#[when("every monty worker is held by another sandbox")]
async fn hold_every_worker(world: &mut World) {
    let workers = thread::available_parallelism().map_or(1, NonZero::get);
    for _ in 0..workers {
        let mut sandbox = world.new_sandbox(None).await.expect("a new sandbox");
        let request = Request::Execute {
            code: "pass".to_string(),
            restart: false,
        };
        let Reply::Status(status) = sandbox.reply(request).await else {
            unreachable!("execute answers a status");
        };
        let status = serde_json::to_value(status).expect("status serializes");
        assert_eq!(status["status"], "finished", "{status}");
        world.other_sandboxes.push(sandbox);
    }
}

#[when("one of the other sandboxes drops")]
fn drop_other_sandbox(world: &mut World) {
    world.other_sandboxes.pop().expect("another sandbox");
}
