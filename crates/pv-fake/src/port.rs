//! Loopback ports that no two running tests share.
//!
//! Ports come in blocks of [`BLOCK_SIZE`] from [`FIRST_PORT`] to [`LAST_PORT`]: below the ports
//! macOS (49152 and up) and Linux (32768 and up) give outgoing connections, and away from PV's own
//! preferred and fallback ports, so PV never picks one of them by itself. A test process owns a
//! block while it holds the exclusive lock on the block's file under the system temp directory, so
//! test processes from concurrent runs, including runs in other worktrees, never share a port. A
//! process keeps its blocks until it exits and never hands out a port twice.

use std::fs::TryLockError;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use socket2::{Domain, Protocol, Socket, Type};
use state::{Database, PortRequest, PvPaths};

const FIRST_PORT: u16 = 20_000;
const LAST_PORT: u16 = 32_767;
const BLOCK_SIZE: u16 = 32;

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

/// A loopback port no other test is handed. It was free for TCP and UDP when it was returned.
pub fn port() -> Result<u16> {
    let mut pool = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = pool.as_mut() {
        return existing.port();
    }

    let directory = Utf8PathBuf::try_from(std::env::temp_dir())?.join("pv-test-ports");
    pool.insert(Pool::new(directory, FIRST_PORT, LAST_PORT))
        .port()
}

/// `COUNT` distinct ports from [`port`].
pub fn ports<const COUNT: usize>() -> Result<[u16; COUNT]> {
    let mut ports = [0; COUNT];
    for slot in &mut ports {
        *slot = port()?;
    }

    Ok(ports)
}

/// Gives the PV home at `paths` a DNS port from [`port`] unless it already has one, and returns
/// the home's DNS port. A daemon that restarts rebinds its saved DNS port without retrying, so the
/// port must be one no other test can take in between.
pub fn seed_dns_port(paths: &PvPaths) -> Result<u16> {
    let dns_port = port()?;
    let assignment = Database::open(paths)?
        .assign_port(PortRequest::dns(dns_port, dns_port, dns_port), |_port| true)?;

    Ok(assignment.port)
}

/// The blocks one process claims from a directory of lock files.
#[expect(
    clippy::disallowed_types,
    reason = "the pool owns its blocks' lock files to keep them locked"
)]
struct Pool {
    directory: Utf8PathBuf,
    first_port: u16,
    last_port: u16,
    next_block: u16,
    /// The ports of the newest block that are not handed out yet.
    ports: Range<u16>,
    locks: Vec<std::fs::File>,
}

impl Pool {
    const fn new(directory: Utf8PathBuf, first_port: u16, last_port: u16) -> Self {
        Self {
            directory,
            first_port,
            last_port,
            next_block: 0,
            ports: 0..0,
            locks: Vec::new(),
        }
    }

    fn port(&mut self) -> Result<u16> {
        loop {
            for port in self.ports.by_ref() {
                if is_free(port).with_context(|| format!("could not probe port {port}"))? {
                    return Ok(port);
                }
            }
            self.claim_block()?;
        }
    }

    fn claim_block(&mut self) -> Result<()> {
        state::fs::ensure_user_dir(&self.directory)?;
        let blocks = (self.last_port - self.first_port + 1) / BLOCK_SIZE;
        while self.next_block < blocks {
            let block_start = self.first_port + self.next_block * BLOCK_SIZE;
            self.next_block += 1;
            let lock_path = self.directory.join(format!("{block_start}.lock"));
            let lock = open_lock_file(&lock_path)?;
            match lock.try_lock() {
                Ok(()) => {
                    self.locks.push(lock);
                    self.ports = block_start..block_start + BLOCK_SIZE;
                    return Ok(());
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("could not lock {lock_path}"));
                }
            }
        }

        bail!(
            "every test port block in {}..={} is held by another test process",
            self.first_port,
            self.last_port
        )
    }
}

/// Opens a block's lock file for writing: Windows can't lock a handle opened only to append.
/// Lock files are never deleted, since a new file at the same path could be locked while another
/// process still holds the old one.
#[expect(
    clippy::disallowed_types,
    reason = "the pool owns its blocks' lock files to keep them locked"
)]
fn open_lock_file(path: &Utf8Path) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("could not open {path}"))
}

/// Whether nothing uses `port` for TCP on loopback or the wildcard address, or for UDP on
/// loopback. The TCP probe sets `SO_REUSEADDR` as runtimes do, so closing connections don't count.
/// Only a port in use counts as busy: any other failure, such as a sandbox refusing to bind, is an
/// error, so it isn't reported as a pool whose every block is taken.
fn is_free(port: u16) -> io::Result<bool> {
    for address in [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED] {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_reuse_address(true)?;
        if in_use(socket.bind(&SocketAddrV4::new(address, port).into()))? {
            return Ok(false);
        }
    }

    Ok(!in_use(
        UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).map(drop),
    )?)
}

/// Whether a bind failed because its port is in use. Any other failure is returned.
fn in_use(bind: io::Result<()>) -> io::Result<bool> {
    match bind {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::process::Stdio;
    use std::thread;

    use anyhow::{Context, Result, anyhow, bail};
    use camino::Utf8PathBuf;
    use camino_tempfile::tempdir;

    use super::{BLOCK_SIZE, FIRST_PORT, LAST_PORT, Pool, open_lock_file, port};

    /// A private pool of two blocks outside the shared pool's ports, so these tests never probe a
    /// port another test was handed.
    const TEST_FIRST_PORT: u16 = 33_000;
    const TEST_LAST_PORT: u16 = TEST_FIRST_PORT + 2 * BLOCK_SIZE - 1;
    const CHILD_POOL_DIRECTORY: &str = "PV_FAKE_TEST_PORT_POOL";
    const CHILD_TEST: &str = "port::tests::holds_a_block_until_its_input_closes";

    #[expect(
        clippy::disallowed_types,
        reason = "the cross-process pool test runs a second test process that holds a block"
    )]
    type TestProcessCommand = std::process::Command;

    #[test]
    fn concurrent_callers_never_get_the_same_port() -> Result<()> {
        let ports = thread::scope(|scope| {
            let callers = (0..8)
                .map(|_caller| scope.spawn(|| (0..10).map(|_port| port()).collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            callers
                .into_iter()
                .map(|caller| {
                    caller
                        .join()
                        .map_err(|_panic| anyhow!("a port caller panicked"))
                })
                .collect::<Result<Vec<_>>>()
        })?
        .into_iter()
        .flatten()
        .collect::<Result<BTreeSet<_>>>()?;

        // 80 ports span at least three blocks.
        assert_eq!(ports.len(), 80);
        assert!(
            ports
                .iter()
                .all(|port| (FIRST_PORT..=LAST_PORT).contains(port))
        );

        Ok(())
    }

    #[test]
    fn a_block_locked_by_another_holder_is_skipped() -> Result<()> {
        let directory = tempdir()?;
        let mut pool = test_pool(directory.path().to_owned());
        let other_holder =
            open_lock_file(&directory.path().join(format!("{TEST_FIRST_PORT}.lock")))?;
        other_holder.try_lock()?;

        let port = pool.port()?;

        assert_eq!(block_start(port), TEST_FIRST_PORT + BLOCK_SIZE);

        Ok(())
    }

    #[test]
    fn a_pool_with_every_block_held_reports_it() -> Result<()> {
        let directory = tempdir()?;
        let mut pool = test_pool(directory.path().to_owned());
        let mut other_holders = Vec::new();
        for block_start in [TEST_FIRST_PORT, TEST_FIRST_PORT + BLOCK_SIZE] {
            let lock = open_lock_file(&directory.path().join(format!("{block_start}.lock")))?;
            lock.try_lock()?;
            other_holders.push(lock);
        }

        let error = pool
            .port()
            .err()
            .ok_or_else(|| anyhow!("expected a full pool to fail"))?;

        assert_eq!(
            error.to_string(),
            "every test port block in 33000..=33063 is held by another test process"
        );

        Ok(())
    }

    #[test]
    fn another_process_holds_its_block_until_it_exits() -> Result<()> {
        let directory = tempdir()?;
        let test_binary = std::env::args_os()
            .next()
            .context("test binary path was missing")?;
        let mut child = TestProcessCommand::new(test_binary)
            .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
            .env(CHILD_POOL_DIRECTORY, directory.path())
            // One test thread makes libtest print the test name before the child's report, on the
            // same line, so the parser always handles that format.
            .env("RUST_TEST_THREADS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut child_output =
            BufReader::new(child.stdout.take().context("child stdout was not piped")?);
        let child_port = read_child_port(&mut child_output)?;

        let mut pool = test_pool(directory.path().to_owned());
        let port_while_child_runs = pool.port()?;
        drop(pool);
        drop(child.stdin.take());
        // Reading to the end lets the child report its result before it exits.
        io::copy(&mut child_output, &mut io::sink())?;
        let child_status = child.wait()?;
        let port_after_child_exits = test_pool(directory.path().to_owned()).port()?;

        assert!(child_status.success());
        assert_eq!(block_start(child_port), TEST_FIRST_PORT);
        assert_eq!(
            block_start(port_while_child_runs),
            TEST_FIRST_PORT + BLOCK_SIZE
        );
        assert_eq!(block_start(port_after_child_exits), TEST_FIRST_PORT);

        Ok(())
    }

    /// Run by `another_process_holds_its_block_until_it_exits` as its second process: claims a
    /// port, reports it, and keeps its block until its input closes.
    #[test]
    #[ignore = "another_process_holds_its_block_until_it_exits runs it"]
    #[expect(
        clippy::disallowed_methods,
        reason = "the parent test passes its pool directory through the environment"
    )]
    fn holds_a_block_until_its_input_closes() -> Result<()> {
        let directory = std::env::var(CHILD_POOL_DIRECTORY)
            .with_context(|| format!("{CHILD_POOL_DIRECTORY} was not set"))?;
        let mut pool = test_pool(Utf8PathBuf::from(directory));
        let port = pool.port()?;

        writeln!(io::stdout(), "port={port}")?;
        io::stdout().flush()?;
        io::stdin().read_to_end(&mut Vec::new())?;

        Ok(())
    }

    fn test_pool(directory: Utf8PathBuf) -> Pool {
        Pool::new(directory, TEST_FIRST_PORT, TEST_LAST_PORT)
    }

    const fn block_start(port: u16) -> u16 {
        port - (port - TEST_FIRST_PORT) % BLOCK_SIZE
    }

    fn read_child_port(child_output: &mut impl BufRead) -> Result<u16> {
        let mut line = String::new();
        while child_output.read_line(&mut line)? > 0 {
            if let Some((_test_name, port)) = line.trim_end().split_once("port=") {
                return Ok(port.parse()?);
            }
            line.clear();
        }

        bail!("child test exited without reporting its port")
    }
}
