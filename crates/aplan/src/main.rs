//! Thin binary shim. All of the CLI lives in the `aplan` library (so it can be
//! introspected and tested); this just hands control to it.

fn main() -> std::process::ExitCode {
    aplan::run_cli()
}
