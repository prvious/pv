use std::io::Write;
use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::Utf8PathBuf;
use daemon::{AdoptedProcess, ManagedProcess, ProcessSpec, ProcessSupervisor};
use pv_fake::{EventKind, FakeSettings, Persona};
use state::{PvPaths, fs};

/// Owns the intended PID/metadata paths before startup and retains captured
/// ownership for cleanup even when a test corrupts or removes those records.
pub struct RuntimeFixture {
    spec: ProcessSpec,
    process: Option<ManagedProcess>,
    ownership: Option<AdoptedProcess>,
    descendant_pid: Option<u32>,
}

impl RuntimeFixture {
    pub fn start(paths: &PvPaths, spec: ProcessSpec) -> Result<Self> {
        let mut fixture = Self {
            spec,
            process: None,
            ownership: None,
            descendant_pid: None,
        };
        let fake = pv_fake::install_with_settings(
            &fixture.spec.command,
            Persona::LongRunning,
            FakeSettings {
                descendant: true,
                ..Default::default()
            },
        )?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let supervisor = ProcessSupervisor::new(paths.clone());
        fixture.process = Some(runtime.block_on(supervisor.start(fixture.spec.clone()))?);
        fixture.ownership = Some(
            supervisor
                .adopt_recorded(&fixture.spec.pid_path, &fixture.spec.metadata_path)?
                .ok_or_else(|| anyhow!("fixture did not publish ownership"))?,
        );
        for _attempt in 0..200 {
            if let Some(pid) = fake
                .events()?
                .into_iter()
                .find_map(|event| match event.kind {
                    EventKind::DescendantSpawned { descendant_pid } => {
                        u32::try_from(descendant_pid).ok()
                    }
                    _ => None,
                })
            {
                fixture.descendant_pid = Some(pid);
                return Ok(fixture);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(anyhow!("fixture descendant did not start"))
    }

    pub fn pid(&self) -> Result<u32> {
        self.process
            .as_ref()
            .map(ManagedProcess::pid)
            .ok_or_else(|| anyhow!("fixture already cleaned"))
    }

    pub fn cleanup(&mut self) -> Result<()> {
        if let Some(ownership) = &self.ownership {
            ownership.kill_and_wait_for_test(Duration::from_secs(2))?;
        } else if self.process.is_some() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            if let Some(process) = self.process.take() {
                runtime.block_on(process.stop(Duration::from_secs(1)))?;
            }
        }
        self.ownership = None;
        self.process = None;
        Ok(())
    }

    pub fn descendant_pid(&self) -> Result<u32> {
        self.descendant_pid
            .ok_or_else(|| anyhow!("fixture descendant did not start"))
    }

    pub fn records_exist(&self) -> Result<bool> {
        Ok(fs::path_entry_exists(&self.spec.metadata_path)?)
    }

    pub fn records_absent(&self) -> Result<bool> {
        Ok(!fs::path_entry_exists(&self.spec.metadata_path)?)
    }
}

/// A monitor record rewritten as another protocol version would write it, which PV refuses to
/// act on. Restore it before fixture cleanup, which finds the runtime through the record.
pub struct ForeignMonitorRecord {
    path: Utf8PathBuf,
    original: String,
}

impl ForeignMonitorRecord {
    pub fn write(paths: &PvPaths, subject: &str) -> Result<Self> {
        let path = paths.monitor_dir(subject).join("monitor.json");
        let original = fs::read_to_string(&path)?;
        let mut record: serde_json::Value = serde_json::from_str(&original)?;
        record["version"] = serde_json::json!(2);
        fs::write_sensitive_file(&path, &serde_json::to_string(&record)?)?;
        Ok(Self { path, original })
    }

    pub fn restore(self) -> Result<()> {
        Ok(fs::write_sensitive_file(&self.path, &self.original)?)
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _reported = writeln!(
                std::io::stderr().lock(),
                "runtime fixture cleanup failed: {error:#}"
            );
        }
    }
}

pub fn gateway_spec(paths: &PvPaths) -> ProcessSpec {
    ProcessSpec {
        name: "gateway".to_owned(),
        command: paths.resources().join("caddy/2/fixture/bin/caddy"),
        arguments: Vec::new(),
        private_environment: Default::default(),
        config_path: paths.gateway_root_config(),
        config_fingerprint: None,
        log_path: paths.gateway_supervisor_log(),
        pid_path: paths.gateway_pid(),
        metadata_path: paths.gateway_runtime_metadata(),
        resource_name: "caddy".to_owned(),
        track: "2".to_owned(),
    }
}
