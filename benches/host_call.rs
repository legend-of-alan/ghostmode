//! Times a script's tool calls through the public `Sandbox` and `Host` API, for a host whose
//! answer future clones each call's arguments and for one whose future clones only the tool id.
//! Run it with `cargo bench --bench host_call`.

use std::hint;
use std::time::{Duration, Instant};

use futures::executor;
use futures::future::{BoxFuture, FutureExt};
use ghostmode::host::{self, Host};
use ghostmode::{Limits, Sandbox, Status};
use serde_json::Value;

/// How many tool calls each timed script makes.
const CALL_COUNT: u32 = 1_000;

/// The sizes of the one string argument each call sends.
const ARG_SIZES_BYTES: [usize; 3] = [100, 10_000, 1_000_000];

/// How many times each script runs. The fastest run is the one reported.
const RUN_COUNT: u32 = 5;

/// The field of a call that a [`CloneHost`] clones into each answer's future.
#[derive(Clone, Copy, Debug)]
enum CallField {
    Args,
    ToolId,
}

/// A host with one tool, `send`, whose answer future owns a clone of one field of the call and
/// answers `null`.
struct CloneHost {
    field: CallField,
}

impl Host for CloneHost {
    fn tools(&self) -> Vec<host::Tool> {
        vec![host::Tool {
            id: "send".into(),
            description: "Takes `data` and answers null.".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }]
    }

    fn call(&self, call: &host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        match self.field {
            CallField::Args => {
                let args = call.args.clone();
                async move {
                    hint::black_box(args);
                    Ok(Value::Null)
                }
                .boxed()
            }
            CallField::ToolId => {
                let tool_id = call.tool_id.clone();
                async move {
                    hint::black_box(tool_id);
                    Ok(Value::Null)
                }
                .boxed()
            }
        }
    }
}

/// The fastest time per call over [`RUN_COUNT`] scripts that each call `send` [`CALL_COUNT`]
/// times with a `data` string of `arg_size_bytes`.
fn measure_time_per_call(field: CallField, arg_size_bytes: usize) -> Duration {
    let script = format!("for _ in range({CALL_COUNT}):\n    send(data=payload)\n");
    let run_times = (0..RUN_COUNT).map(|_| {
        let limits = Limits {
            calls_per_execute: CALL_COUNT as usize,
            ..Limits::default()
        };
        let mut sandbox = Sandbox::builder(CloneHost { field }).limits(limits).build();
        expect_finished(executor::block_on(
            sandbox.execute(&format!("payload = 'x' * {arg_size_bytes}"), false),
        ));
        let started = Instant::now();
        let status = executor::block_on(sandbox.execute(&script, false));
        let run_time = started.elapsed();
        expect_finished(status);
        run_time
    });
    run_times.min().expect("at least one run") / CALL_COUNT
}

/// Panics unless the script finished.
fn expect_finished(status: Status) {
    assert!(
        matches!(status, Status::Finished { .. }),
        "the script did not finish: {status:?}"
    );
}

fn main() {
    println!("time per call over {CALL_COUNT} calls, fastest of {RUN_COUNT} runs");
    println!(
        "{:>12} {:>16} {:>16}",
        "arg size", "clone args", "clone tool id"
    );
    for arg_size_bytes in ARG_SIZES_BYTES {
        let args_time = measure_time_per_call(CallField::Args, arg_size_bytes);
        let tool_id_time = measure_time_per_call(CallField::ToolId, arg_size_bytes);
        let size = format!("{arg_size_bytes} B");
        println!(
            "{size:>12} {:>16} {:>16}",
            format!("{args_time:.2?}"),
            format!("{tool_id_time:.2?}")
        );
    }
}
