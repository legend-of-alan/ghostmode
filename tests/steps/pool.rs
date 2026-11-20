use std::process::{self, Command};

use cucumber::when;

use crate::World;

/// Kills every `monty` worker this test process started. The test pool keeps no spare worker,
/// so the only one alive is the one running the session's script.
#[when("every monty worker process is killed")]
fn kill_workers(_world: &mut World) {
    let mut kill = if cfg!(windows) {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/F", "/IM", "monty.exe"]);
        taskkill
    } else {
        let mut pkill = Command::new("pkill");
        let parent = process::id().to_string();
        pkill.args(["-KILL", "-P", &parent, "-x", "monty"]);
        pkill
    };
    let status = kill.status().expect("the process killer runs");
    assert!(status.success(), "{status}");
}
