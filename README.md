# ghostmode

Code Mode for Rust agents. A model writes a short script, ghostmode runs it in a sandbox, and only the result comes back. The model sees two tools, `execute` and `poll`. Your own functions are exposed inside the script, and the core is async without tying you to a runtime.

Early days: the design is underway and the first real release is coming. The `0.0.1` crate on crates.io only reserves the name.

Licensed under MIT.
