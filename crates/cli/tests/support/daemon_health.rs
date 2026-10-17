use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::json;
use state::{PvPaths, fs};

/// Serves one health request on the test home's socket, with bounded I/O and joined teardown.
#[derive(Debug)]
pub struct DaemonFixture {
    thread: Option<JoinHandle<Result<Vec<String>>>>,
}

impl DaemonFixture {
    pub fn start(paths: &PvPaths) -> Result<Self> {
        fs::ensure_layout(paths)?;
        fs::remove_file_if_exists(&paths.daemon_socket())?;
        let listener = UnixListener::bind(paths.daemon_socket())?;
        listener.set_nonblocking(true)?;
        let thread = thread::Builder::new()
            .name("dns-cli-health".to_owned())
            .spawn(move || {
                let started = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == io::ErrorKind::WouldBlock
                                && started.elapsed() < Duration::from_secs(3) =>
                        {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request)?;
                writeln!(
                    stream,
                    "{}",
                    json!({
                        "type": "response",
                        "protocol_version": daemon::PROTOCOL_VERSION,
                        "status": "ok",
                        "message": "daemon healthy",
                    })
                )?;
                Ok(vec![request.trim().to_owned()])
            })?;
        Ok(Self {
            thread: Some(thread),
        })
    }

    pub fn finish(mut self) -> Result<Vec<String>> {
        self.join()
    }

    fn join(&mut self) -> Result<Vec<String>> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow!("daemon health fixture thread failed"))?
        } else {
            Ok(Vec::new())
        }
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        if let Err(error) = self.join() {
            let _reported = writeln!(
                io::stderr().lock(),
                "daemon health fixture cleanup failed: {error:#}"
            );
        }
    }
}
