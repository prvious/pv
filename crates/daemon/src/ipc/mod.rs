#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub(crate) use self::unix::{LocalListener, LocalStream, bind, prepare_endpoint, remove_endpoint};

#[cfg(not(unix))]
compile_error!("PV daemon IPC currently supports Unix platforms only");
