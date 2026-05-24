use std::ffi::OsString;

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;
}

#[derive(Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        process_var_os(key)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV environment helper owns direct process environment reads"
)]
fn process_var_os(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}
