use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use platform::{KeychainCertificate, KeychainTrustResult, PlatformError, generate_local_ca};
use state::{PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    certificates: Vec<KeychainCertificate>,
    keychain_error: Option<String>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            certificates: Vec::new(),
            keychain_error: None,
        }
    }

    fn with_certificate(mut self, certificate: KeychainCertificate) -> Self {
        self.certificates.push(certificate);
        self
    }

    fn with_keychain_error(mut self, message: &str) -> Self {
        self.keychain_error = Some(message.to_string());
        self
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        if let Some(message) = &self.keychain_error {
            return Err(PlatformError::Keychain(message.clone()));
        }

        Ok(self.certificates.clone())
    }
}

#[test]
fn ca_trust_generates_local_ca_and_defers_system_keychain_trust() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate = read_required_file(&paths.ca_certificate())?;
    let private_key = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(certificate.contains("BEGIN CERTIFICATE"));
    assert!(private_key.contains("BEGIN PRIVATE KEY"));

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, paths.ca_certificate(), paths.ca_private_key()));
    });

    Ok(())
}

#[test]
fn ca_trust_reuses_existing_current_local_ca() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);
    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate_after = read_required_file(&paths.ca_certificate())?;
    let private_key_after = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_eq!(certificate_after, generated.certificate_pem);
    assert_eq!(private_key_after, generated.private_key_pem);

    Ok(())
}

#[test]
fn ca_trust_repairs_malformed_local_ca_files() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);
    write_file(&paths.ca_certificate(), "not a certificate\n")?;
    write_file(&paths.ca_private_key(), "not a private key\n")?;

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate_after = read_required_file(&paths.ca_certificate())?;
    let private_key_after = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(certificate_after.contains("BEGIN CERTIFICATE"));
    assert!(private_key_after.contains("BEGIN PRIVATE KEY"));

    Ok(())
}

#[test]
fn ca_status_reports_local_and_system_trust_without_creating_files() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let paths = pv_paths(&home);
    let missing_environment = TestEnvironment::new(&home, &current_dir);

    let missing = run_pv(&["ca:status"], &missing_environment)?;
    let certificate_after_missing = read_optional_file(&paths.ca_certificate())?;
    let key_after_missing = read_optional_file(&paths.ca_private_key())?;

    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let current_environment =
        TestEnvironment::new(&home, &current_dir).with_certificate(KeychainCertificate {
            metadata: generated.metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        });
    let current = run_pv(&["ca:status"], &current_environment)?;

    let unreadable_environment =
        TestEnvironment::new(&home, &current_dir).with_keychain_error("fixture keychain failure");
    let unreadable = run_pv(&["ca:status"], &unreadable_environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(unreadable.exit_code, ExitCode::SUCCESS);
    assert!(certificate_after_missing.is_none());
    assert!(key_after_missing.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((missing, current, unreadable));
    });

    Ok(())
}

#[test]
fn ca_untrust_leaves_local_ca_files_and_defers_system_keychain_removal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let paths = pv_paths(&home);
    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let environment =
        TestEnvironment::new(&home, &current_dir).with_certificate(KeychainCertificate {
            metadata: generated.metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        });

    let output = run_pv(&["ca:untrust"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.ca_certificate())?.is_some());
    assert!(read_optional_file(&paths.ca_private_key())?.is_some());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn ca_untrust_succeeds_when_system_trust_is_absent_and_preserves_local_ca_files()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let paths = pv_paths(&home);
    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let environment = TestEnvironment::new(&home, &current_dir);

    let output = run_pv(&["ca:untrust"], &environment)?;
    let certificate_after = read_required_file(&paths.ca_certificate())?;
    let private_key_after = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_no_privileged_guidance(&output.stdout);
    assert_eq!(certificate_after, generated.certificate_pem);
    assert_eq!(private_key_after, generated.private_key_pem);

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

fn assert_no_privileged_guidance(output: &str) {
    for pattern in ["sudo", "security ", "security\n", "openssl"] {
        assert!(
            !output.contains(pattern),
            "output contains privileged guidance `{pattern}`: {output}"
        );
    }
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"[a-f0-9]{64}", "<fingerprint>");
    settings.bind(assertion);
}
