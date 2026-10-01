//! Test controls and records for the Gateway personas, kept next to the runtime config. Tests
//! steer a running fake through `fake-admin-control.json` and inspect what it received through the
//! `fake-admin-*` records.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use hyper::Method;
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CONTROL_FILE: &str = "fake-admin-control.json";
const CONTROL_LOCK_FILE: &str = "fake-admin-control.lock";
const REQUESTS_FILE: &str = "fake-admin-requests.jsonl";
const CURRENT_CONFIG_FILE: &str = "fake-admin-current.bin";
const VALIDATOR_SPAWNS_FILE: &str = "fake-validator-spawns.log";

/// Test controls for a running Gateway fake. A list is used up one entry per request; a single
/// value applies to every request. Nothing set means recorded Caddy behavior.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Control {
    /// Statuses for `GET /config/`.
    admin_statuses: Option<Setting<u16>>,
    /// Holds `GET /config/` until this path exists.
    admin_response_gate: Option<Setting<Utf8PathBuf>>,
    /// Statuses for `POST /load`; anything but 2xx rejects the load without applying it.
    load_statuses: Option<Setting<u16>>,
    /// `false` accepts a load without applying it.
    apply_load: Option<Setting<bool>>,
    /// `true` applies a load but keeps the previous listeners, so changed ports never open.
    retain_previous_listeners: Option<Setting<bool>>,
    /// Holds `POST /load`, after recording it, until this path exists.
    load_response_gate: Option<Setting<Utf8PathBuf>>,
    /// Delays the `/load` response.
    load_delay_ms: Option<Setting<u64>>,
    /// With `late_apply_delay_ms`, applies the load that long after it arrived, even when the
    /// client gave up waiting.
    late_accept: Option<Setting<bool>>,
    late_apply_delay_ms: Option<Setting<u64>>,
    /// Replaces the `/load` response body.
    load_response_body: Option<Setting<String>>,
    /// Written once a load is accepted.
    load_accepted_marker: Option<Setting<Utf8PathBuf>>,
    /// Exits 0 shortly after accepting a load.
    exit_after_load: Option<Setting<bool>>,
    /// Closes the HTTP listener on its next connection and keeps the process running.
    pub(crate) stop_service: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
enum Setting<T> {
    Queue(Vec<T>),
    Always(T),
}

fn next<T: Clone>(setting: &mut Option<Setting<T>>) -> Option<T> {
    match setting {
        Some(Setting::Queue(values)) if !values.is_empty() => Some(values.remove(0)),
        Some(Setting::Always(value)) => Some(value.clone()),
        _ => None,
    }
}

/// What the controls say to do with one `GET /config/`.
pub(crate) struct AdminControl {
    pub(crate) status: u16,
    pub(crate) gate: Option<Utf8PathBuf>,
}

impl AdminControl {
    pub(crate) fn take(control: &mut Control) -> Self {
        Self {
            status: next(&mut control.admin_statuses).unwrap_or(200),
            gate: next(&mut control.admin_response_gate),
        }
    }
}

/// What the controls say to do with one `POST /load`. Settings that only matter for an accepted
/// load are used up only by accepted loads.
pub(crate) struct LoadControl {
    /// The response status: `400` for a config the adapter couldn't read, as with Caddy, and
    /// otherwise `load_statuses`.
    pub(crate) status: u16,
    pub(crate) apply: bool,
    pub(crate) retain_listeners: bool,
    pub(crate) gate: Option<Utf8PathBuf>,
    pub(crate) delay: Duration,
    pub(crate) late_accept: bool,
    pub(crate) late_apply_delay: Duration,
    pub(crate) response_body: Option<String>,
    pub(crate) accepted_marker: Option<Utf8PathBuf>,
    pub(crate) exit_after: bool,
}

impl LoadControl {
    pub(crate) fn take(control: &mut Control, adapted: bool) -> Self {
        let status = next(&mut control.load_statuses).unwrap_or(200);
        let status = if adapted { status } else { 400 };
        let accepted = is_success(status);

        Self {
            status,
            apply: accepted && next(&mut control.apply_load).unwrap_or(true),
            retain_listeners: accepted
                && next(&mut control.retain_previous_listeners).unwrap_or(false),
            gate: next(&mut control.load_response_gate),
            delay: Duration::from_millis(next(&mut control.load_delay_ms).unwrap_or(0)),
            late_accept: next(&mut control.late_accept).unwrap_or(false),
            late_apply_delay: Duration::from_millis(
                next(&mut control.late_apply_delay_ms).unwrap_or(0),
            ),
            response_body: next(&mut control.load_response_body),
            accepted_marker: accepted
                .then(|| next(&mut control.load_accepted_marker))
                .flatten(),
            exit_after: accepted && next(&mut control.exit_after_load).unwrap_or(false),
        }
    }

    pub(crate) fn accepted(&self) -> bool {
        is_success(self.status)
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Writes a Gateway fake's test controls next to `config_path`, rejecting keys the fake doesn't
/// know. Keys and values follow `fake-admin-control.json`, e.g. `{"load_statuses": [422]}`.
pub fn write_gateway_control(config_path: &Utf8Path, control: Value) -> Result<()> {
    let control: Control =
        serde_json::from_value(control).context("invalid Gateway fake control")?;
    let records = Records::beside(config_path)?;

    records.with_control_lock(|| records.write_control(&control))
}

/// The control file and records in one runtime config's directory.
#[derive(Debug)]
pub(crate) struct Records {
    directory: Utf8PathBuf,
    /// Numbers `fake-admin-load-NNN.bin` from 0 in each process, so a replacement runtime
    /// overwrites the first bodies.
    loads: AtomicUsize,
}

impl Records {
    pub(crate) fn beside(config_path: &Utf8Path) -> Result<Self> {
        let directory = config_path
            .parent()
            .ok_or_else(|| anyhow!("{config_path} has no parent directory"))?;

        Ok(Self {
            directory: directory.to_owned(),
            loads: AtomicUsize::new(0),
        })
    }

    pub(crate) fn read_control(&self) -> Result<Control> {
        let path = self.directory.join(CONTROL_FILE);
        if !state::fs::path_entry_exists(&path)? {
            return Ok(Control::default());
        }

        serde_json::from_str(&state::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {path}"))
    }

    /// Reads the controls, applies `take`, and writes back what it used up, holding the control
    /// lock so a test's write in between isn't lost.
    pub(crate) fn take_control<R>(&self, take: impl FnOnce(&mut Control) -> R) -> Result<R> {
        self.with_control_lock(|| {
            let mut control = self.read_control()?;
            let before = control.clone();
            let taken = take(&mut control);
            if control != before {
                self.write_control(&control)?;
            }

            Ok(taken)
        })
    }

    fn with_control_lock<R>(&self, locked: impl FnOnce() -> Result<R>) -> Result<R> {
        let lock = state::fs::open_append_file(&self.directory.join(CONTROL_LOCK_FILE))?;
        flock(&lock, FlockOperation::LockExclusive)?;

        locked()
    }

    fn write_control(&self, control: &Control) -> Result<()> {
        let mut value = serde_json::to_value(control)?;
        if let Value::Object(settings) = &mut value {
            settings.retain(|_key, setting| !setting.is_null());
        }
        state::fs::write_sensitive_file(
            &self.directory.join(CONTROL_FILE),
            &serde_json::to_string(&value)?,
        )?;

        Ok(())
    }

    pub(crate) fn request(
        &self,
        method: &Method,
        path: &str,
        status: u16,
        body_length: usize,
    ) -> Result<()> {
        let mut line = json!({
            "body_length": body_length,
            "method": method.as_str(),
            "path": path,
            "status": status,
        })
        .to_string();
        line.push('\n');
        state::fs::open_append_file(&self.directory.join(REQUESTS_FILE))?
            .write_all(line.as_bytes())?;

        Ok(())
    }

    pub(crate) fn load(&self, body: &str) -> Result<()> {
        let number = self.loads.fetch_add(1, Ordering::Relaxed);
        state::fs::write_sensitive_file(
            &self
                .directory
                .join(format!("fake-admin-load-{number:03}.bin")),
            body,
        )?;

        Ok(())
    }

    /// Records the config the runtime serves: the initial config at startup, then each applied
    /// load.
    pub(crate) fn current(&self, config: &str) -> Result<()> {
        state::fs::write_sensitive_file(&self.directory.join(CURRENT_CONFIG_FILE), config)?;

        Ok(())
    }
}

/// Appends a validated config's path to `fake-validator-spawns.log` next to it.
pub(crate) fn record_validation(config_path: &Utf8Path) -> Result<()> {
    let records = Records::beside(config_path)?;
    state::fs::open_append_file(&records.directory.join(VALIDATOR_SPAWNS_FILE))?
        .write_all(format!("{config_path}\n").as_bytes())?;

    Ok(())
}
