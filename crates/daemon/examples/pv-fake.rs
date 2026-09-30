//! Builds the `pv-fake` test runtime along with the daemon's tests; see `pv_fake::binary`.

use std::process::ExitCode;

fn main() -> ExitCode {
    pv_fake::main()
}
