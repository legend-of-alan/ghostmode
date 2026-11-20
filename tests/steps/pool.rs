use std::process::{self, Command};

use cucumber::when;

use crate::World;

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
