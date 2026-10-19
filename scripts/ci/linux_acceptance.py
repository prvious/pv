"""Run the Linux support gate on a disposable machine as its ordinary test user."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
HOME = Path.home()
PV_HOME = HOME / ".pv"
INPUTS = Path(os.environ["PV_ACCEPTANCE_INPUT_DIR"])
EVIDENCE = Path(os.environ["PV_ACCEPTANCE_EVIDENCE_DIR"])
APP = INPUTS / "app"
RELEASE = APP / "pv-release"
PV = PV_HOME / "bin/pv"
PLATFORM = "linux-arm64" if os.uname().machine == "aarch64" else "linux-amd64"
EXPECTED = {
    "php": {"8.3", "8.4", "8.5"}, "frankenphp": {"8.3", "8.4", "8.5"},
    "mysql": {"8.0", "8.4", "9.7"}, "postgres": {"17", "18"},
    "redis": {"8.8"}, "caddy": {"2"}, "mailpit": {"1"},
    "rustfs": {"1"}, "composer": {"2"},
}


def run(label, *command, environment=None, timeout=120, cwd=None):
    print(f"Checking {label}", flush=True)
    result = subprocess.run(command, env=environment, cwd=cwd, timeout=timeout,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    (EVIDENCE / f"{label}.log").write_bytes(result.stdout)
    if result.returncode:
        raise RuntimeError(f"{label} failed ({result.returncode}); see {label}.log")
    return result.stdout.decode()


def wait_for(label, check, seconds=120):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(1)
    raise RuntimeError(f"{label} timed out after {seconds}s")


def runtime_pids():
    records = {}
    for path in (PV_HOME / "run").rglob("*.json"):
        record = json.loads(path.read_text())
        if "pid" in record and "command" in record:
            pid = record["pid"]
            assert Path(f"/proc/{pid}/exe").exists(), f"runtime is absent: {path}"
            records[str(path.relative_to(PV_HOME))] = pid
    assert len(records) >= 7, "expected Gateway, PHP and all five resources"
    return records


def pv_processes():
    processes = []
    for path in Path("/proc").glob("[0-9]*/exe"):
        try:
            executable = os.readlink(path)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if executable.startswith(str(PV_HOME) + "/"):
            processes.append(path.parent.name)
    return processes


def prepare_artifacts():
    records = {}
    for path in INPUTS.rglob("*.json"):
        value = json.loads(path.read_text())
        if isinstance(value, dict) and "resource" in value and "object_key" in value:
            if value["platform"] in (PLATFORM, "any"):
                identity = value["resource"], value["track"]
                assert identity not in records, f"duplicate artifact: {identity}"
                records[identity] = (path, value)
    expected = {(resource, track) for resource, tracks in EXPECTED.items() for track in tracks}
    assert set(records) == expected, f"artifact set differs: missing={expected - records.keys()}"
    records_dir = EVIDENCE / "records"
    records_dir.mkdir()
    downloads = PV_HOME / "downloads"
    downloads.mkdir(parents=True, mode=0o700)
    archives = {}
    for identity, (record_path, record) in records.items():
        filename = Path(record["object_key"]).name
        matches = list(INPUTS.rglob(filename))
        assert len(matches) == 1, f"expected one archive: {filename}"
        archive = matches[0]
        assert archive.stat().st_size == record["size"], f"size mismatch: {filename}"
        digest = hashlib.sha256()
        with archive.open("rb") as source:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(block)
        assert digest.hexdigest() == record["sha256"], f"checksum mismatch: {filename}"
        shutil.copyfile(record_path, records_dir / record_path.name)
        shutil.copyfile(archive, downloads / f"{record['sha256']}-{filename}")
        archives[identity] = archive
        run(f"layout-{identity[0]}-{identity[1]}", str(RELEASE), "validate-archive",
            "--archive", str(archive), "--record", str(record_path))
    # Composer smoke uses the actual Linux PHP artifact, independent of distro PHP packages.
    php_directory = EVIDENCE / "smoke-php"
    php_directory.mkdir()
    run("extract-smoke-php", "tar", "-xzf", str(archives["php", "8.5"]), "-C", str(php_directory))
    php = next(php_directory.glob("*/bin/php"))
    for (resource, track), (record_path, record) in records.items():
        environment = os.environ.copy()
        environment["PV_UPSTREAM_VERSION"] = record["upstream_version"]
        environment["PV_COMPOSER_SMOKE_PHP"] = str(php)
        hook_resource = "php" if resource == "frankenphp" else resource
        if resource in ("php", "frankenphp"):
            text = run(f"php-inputs-{resource}-{track}", str(RELEASE), "print-recipe-env",
                       "--php", str(ROOT / "release/artifacts/recipes/php/tracks.toml"),
                       "--resource", resource, "--track", track, "--platform", PLATFORM)
            for line in text.splitlines():
                name, value = line.split("=", 1)
                environment[name] = shlex.split(value)[0]
        if resource == "postgres":
            environment["PV_POSTGRES_PLATFORM"] = PLATFORM
            environment["PV_POSTGRES_EXTENSION_CATALOG"] = str(
                ROOT / f"release/artifacts/recipes/postgres/catalogs/{track}.txt")
        run(f"smoke-{resource}-{track}", str(RELEASE), "validate-archive",
            "--archive", str(archives[resource, track]), "--record", str(record_path),
            "--smoke-hook", str(ROOT / f"release/artifacts/recipes/{hook_resource}/smoke.sh"),
            environment=environment, timeout=180)
    run("generate-manifest", str(RELEASE), "generate-manifest", "--records", str(records_dir),
        "--revocations", str(ROOT / "release/artifacts/revocations"),
        "--defaults", str(ROOT / "release/artifacts/default-tracks.toml"),
        "--base-url", "https://artifacts.pv-linux.invalid", "--output", str(downloads / "manifest.json"))


def install():
    installer = EVIDENCE / "install.sh"
    base = "https://artifacts.pv-linux.invalid"
    run("generate-installer", str(RELEASE), "generate-app-installer", "--records",
        str(APP / "records"), "--base-url", base, "--output", str(installer))
    # Only fixture asset URLs change. The generated install/checksum/setup body runs intact.
    installer.write_text(installer.read_text().replace(base, INPUTS.as_uri()))
    run("install", "bash", str(installer), "--yes", "--no-setup", timeout=180)
    run("setup", str(PV), "setup", "--yes", timeout=600)


def https_ready():
    result = subprocess.run(["curl", "--noproxy", "*", "--fail", "--silent", "--show-error",
                             "--max-time", "5", "https://linux-acceptance.test/"],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if result.returncode:
        return False
    body = json.loads(result.stdout)
    assert body["php"].startswith("8.5.")
    assert body["mysql"] == body["postgres"] == 1
    assert body["redis"] and body["mailpit"] and body["rustfs"]
    (EVIDENCE / "project.json").write_text(json.dumps(body, indent=2))
    return True


def system_acceptance():
    run("helper-active", "systemctl", "is-active", "pv-helper.socket")
    run("dns-active", "systemctl", "is-active", "pv-dns.service")
    assert "127.0.0.1" in run("getaddrinfo", "getent", "ahostsv4", "probe.test")
    run("resolved-query", "resolvectl", "query", "probe.test")
    run("daemon-active", "systemctl", "--user", "is-active", "pv-daemon.service")
    trust = run("browser-trust", "certutil", "-d", f"sql:{HOME / '.pki/nssdb'}", "-L")
    assert "PV Local Development CA " in trust and "C,," in trust
    project = EVIDENCE / "project"
    (project / "public").mkdir(parents=True)
    shutil.copyfile(ROOT / "scripts/ci/linux-project.yml", project / "pv.yml")
    shutil.copyfile(ROOT / "scripts/ci/linux-project.php", project / "public/index.php")
    run("link", str(PV), "link", str(project), "--hostname", "linux-acceptance.test", timeout=300)
    wait_for("system-trusted HTTPS and resources", https_ready)
    wildcard = run("wildcard", "curl", "--noproxy", "*", "-fsS", "--max-time", "10",
                   "https://tenant.linux-acceptance.test/")
    assert json.loads(wildcard)["mysql"] == 1
    redirect = run("http-redirect", "curl", "--noproxy", "*", "-sSI", "--max-time", "10",
                   "http://linux-acceptance.test/")
    assert "https://linux-acceptance.test/" in redirect
    run("doctor", str(PV), "doctor", "--json")
    before = runtime_pids()
    run("daemon-restart", str(PV), "daemon:restart")
    wait_for("HTTPS after daemon restart", https_ready)
    assert runtime_pids() == before, "daemon restart replaced a managed runtime"
    environment = os.environ.copy()
    for line in (project / ".env").read_text().splitlines():
        if line.startswith("POSTGRES_URL="):
            environment["PGDATABASE"] = shlex.split(line.split("=", 1)[1])[0]
    postgres_pid = runtime_pids()["run/resources/postgres/18.json"]
    psql = next((PV_HOME / "resources/postgres/18").glob("*/bin/psql"))
    with (EVIDENCE / "open-postgres-client.log").open("wb") as output:
        client = subprocess.Popen([str(psql), environment["PGDATABASE"], "-X", "-c", "SELECT pg_sleep(120)"],
                                  env=environment, stdout=output, stderr=subprocess.STDOUT)
        try:
            time.sleep(1)
            assert client.poll() is None, "Postgres client failed before stop"
            started = time.monotonic()
            run("unlink-with-open-client", str(PV), "unlink", "linux-acceptance.test", timeout=30)
            client.wait(timeout=10)
            assert not Path(f"/proc/{postgres_pid}").exists(), "Postgres remains after stop"
            assert time.monotonic() - started < 20, "Postgres stop was not fast"
        finally:
            if client.poll() is None:
                client.terminate()
                client.wait(timeout=10)
    run("relink", str(PV), "link", str(project), "--hostname", "linux-acceptance.test", timeout=300)
    wait_for("resources after relink", https_ready)
    previous = runtime_pids()
    manager = f"user@{os.getuid()}.service"
    run("logout", "sudo", "-n", "systemctl", "stop", manager)
    wait_for("all PV processes stopped at logout", lambda: not pv_processes(), 30)
    run("login", "sudo", "-n", "systemctl", "start", manager)
    wait_for("resources after login", https_ready)
    assert runtime_pids() != previous, "login did not recreate stopped runtimes"
    public_ca = "".join((PV_HOME / "certificates/ca.pem").read_text().splitlines()[1:-1])
    run("uninstall", str(PV), "uninstall", "--prune", "--force", timeout=180)
    assert not PV_HOME.exists(), "PV home remains after prune"
    assert not pv_processes(), "PV processes remain after uninstall"
    for path in ["/usr/local/libexec/pv", "/var/lib/pv", "/run/pv-helper.sock",
                 str(HOME / ".config/systemd/user/pv-daemon.service")]:
        assert not Path(path).exists(), f"PV installation remains: {path}"
    for directory in ["/etc/systemd/system", "/run/systemd/system"]:
        assert not list(Path(directory).rglob("pv-*.service")), "PV service remains"
        assert not list(Path(directory).rglob("pv-*.socket")), "PV socket remains"
    for directory in ["/usr/local/share/ca-certificates", "/etc/pki/ca-trust/source/anchors"]:
        assert not list(Path(directory).glob("pv-local-ca-*")), "PV CA anchor remains"
    for bundle in ["/etc/ssl/certs/ca-certificates.crt",
                   "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem"]:
        if Path(bundle).exists():
            assert public_ca not in "".join(Path(bundle).read_text().split()), "PV CA remains trusted"
    assert "pv0" not in run("links-after-uninstall", "ip", "-j", "link", "show")
    trust = run("browser-after-uninstall", "certutil", "-d", f"sql:{HOME / '.pki/nssdb'}", "-L")
    assert "PV Local Development CA " not in trust


def main():
    assert os.environ.get("PV_LINUX_ACCEPTANCE") == "1", "requires an explicit disposable-machine run"
    assert sys.platform == "linux" and os.getuid() != 0
    assert Path("/proc/1/comm").read_text().strip() == "systemd"
    assert not PV_HOME.exists(), "refusing an existing PV home"
    assert not Path("/var/lib/pv").exists(), "refusing an existing helper"
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    try:
        run("user-manager", "systemctl", "--user", "show-environment")
        run("resolved-active", "systemctl", "is-active", "systemd-resolved.service")
        assert shutil.which("certutil"), "certutil is required"
        prepare_artifacts()
        install()
        system_acceptance()
        result = {"platform": PLATFORM, "artifact_smoke": "passed", "system_acceptance": "passed"}
    except Exception as error:
        result = {"platform": PLATFORM, "result": "failed", "reason": str(error)}
        raise
    finally:
        (EVIDENCE / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print("Linux support gate passed", flush=True)


if __name__ == "__main__":
    main()
