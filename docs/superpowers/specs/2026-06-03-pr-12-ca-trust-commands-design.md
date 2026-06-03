# PR 12 CA Trust Commands Design

## Goal

Implement roadmap PR 12, covering `PV-054`, without pulling in later setup, uninstall, LaunchAgent, Gateway runtime, or real privileged System keychain mutation work.

PR 12 should deliver PV's local CA preparation and status surface. It should generate and repair PV's local CA files under `~/.pv/certificates/`, inspect those local files, inspect macOS System keychain trust read-only, and expose useful `pv ca:*` commands. It must not run `sudo`, invoke the `security` CLI, or mutate the System keychain.

## Scope

Roadmap PR 12 covers:

| Package | Purpose |
| --- | --- |
| `PV-054` | Implement local CA file generation and lower-level `pv ca:*` trust commands. |

This PR depends on the base layout and CLI foundations from PR 2 and PR 4. It follows the PR 10 and PR 11 system-integration boundary: generate and inspect now, leave foreground privileged mutation to PR 13 setup/system-integration work.

## Decisions

Use a prepared-local-CA approach for PR 12. PV will render the local root certificate and private key under `~/.pv/certificates/`, then report how the local CA maps to System keychain trust. Later setup code can install or remove System keychain trust with the required privileges.

Defer real System keychain trust mutation. `pv ca:trust` should generate or repair local CA files, inspect System keychain trust read-only, and exit non-zero with clear guidance that privileged trust installation is deferred. `pv ca:untrust` should inspect local CA files and System keychain trust read-only, leave CA files in place, and exit non-zero when PV-owned trust appears to remain or cannot be inspected safely.

Use Rust libraries rather than shelling out. `rcgen` should own certificate and private-key generation. `rustls-pemfile` should own PEM decoding. `x509-parser` should own X.509 validation for existing certificate files. `sha2` should own DER SHA-256 fingerprint calculation. `security-framework` should own read-only macOS trust inspection. Command handlers must not spawn `security`, `openssl`, or any other external process.

## Architecture

The `state` crate should own path derivation for PV CA files:

- `~/.pv/certificates/ca.pem`;
- `~/.pv/certificates/ca-key.pem`.

The `macos` crate should own CA domain logic:

- generate a PV-owned root CA certificate and private key;
- parse and inspect the local CA certificate and key files;
- classify local CA files as missing, current, repair-required, mismatch, or unreadable;
- compute the local CA certificate fingerprint from DER bytes;
- inspect System keychain trust read-only by comparing trusted certificate fingerprints to the local CA fingerprint;
- classify trust as current, not trusted, stale, denied, unknown, or unreadable.

The CLI should add `ca:status`, `ca:trust`, and `ca:untrust`. These commands should use state and macOS helpers, support injected CA paths and injected keychain inspectors in tests, and avoid privileged mutation.

## Local CA Files

PV should generate one user-specific root CA:

| File | Purpose |
| --- | --- |
| `ca.pem` | PEM-encoded self-signed PV local root CA certificate. |
| `ca-key.pem` | PEM-encoded private key for the PV local root CA. |

The certificate should have a stable PV identity:

- common name: `PV Local Development CA`;
- organization: `PV`;
- CA basic constraints enabled;
- key usage suitable for signing Project certificates;
- no Project hostnames in the root certificate;
- a 10-year validity period suitable for local development.

The private key should be generated as an ECDSA P-256 key using `rcgen`'s `PKCS_ECDSA_P256_SHA256` signing algorithm. The file format should be PEM so the future Gateway/Caddy/FrankenPHP integration can consume it without an additional conversion step.

Local CA files are user-owned generated config. `ca:trust` may overwrite them when they are missing, malformed, mismatched, or otherwise unusable. If one of the pair is valid and the other is missing or mismatched, PV should generate a new pair rather than trying to patch only one side. Regenerating the pair may leave old System keychain trust behind; PR 12 should report that condition when detectable and leave removal to PR 13.

## System Keychain Trust

System keychain inspection should be read-only. PV should compare the DER SHA-256 fingerprint of the local CA certificate against certificates with trust settings in the macOS administered System trust domain. A fingerprint match with trust-root or trust-as-root settings is current. A missing match is not trusted.

If the keychain contains a PV-looking CA with the expected subject but a different fingerprint, status should report stale PV CA trust. If the keychain reports an explicit deny for the local CA fingerprint, status should report denied. If local CA files are missing or unreadable, keychain trust status should be unknown because PV cannot safely correlate the intended local CA.

PR 12 should not import certificates into the keychain, change trust settings, or delete trusted certificates. The future mutation hooks should be named precisely in comments or typed interfaces so PR 13 can wire them without reworking status classification.

## Command Behavior

`pv ca:status` should be read-only. It must not generate CA files, create directories, delete files, mutate keychain trust, run external commands, or mutate state. It should report local CA file status and System keychain trust status independently.

`pv ca:trust` should:

1. resolve PV paths from the injected home;
2. inspect local CA files;
3. generate a new local CA pair when the local files are missing, malformed, mismatched, or unusable;
4. reuse the existing local CA pair when it is current;
5. inspect System keychain trust read-only using the local CA fingerprint;
6. exit non-zero with clear guidance that privileged trust installation is deferred to PR 13 setup/system-integration work.

`pv ca:untrust` should:

1. inspect local CA files without deleting them;
2. inspect System keychain trust read-only when local CA correlation is possible;
3. report whether PV-owned System trust appears to remain;
4. exit success only when no current PV trust is present and no stale PV-looking trust is detected;
5. exit non-zero when current trust, stale trust, denied trust, unreadable trust, or unknown trust remains.

These commands should use human output only for PR 12. JSON output can wait for the later `PV-095` JSON outputs package.

## Status States

`ca:status` should report two independent state groups:

- local CA files;
- System keychain trust.

Local CA states should include:

- missing: neither local CA file exists;
- current: certificate and key parse, have the expected PV CA shape, and match each other;
- repair-required: exactly one file is missing, the pair is malformed, the pair is mismatched, or the certificate shape is not a usable PV CA;
- unreadable: either path cannot be read.

System trust states should include:

- current: the local CA fingerprint is trusted as a root;
- not trusted: the local CA fingerprint is not trusted and no stale PV-looking trust is detected;
- stale: a PV-looking CA with a different fingerprint is trusted;
- denied: the local CA fingerprint is explicitly distrusted;
- unknown: local CA status prevents safe correlation;
- unreadable: keychain inspection failed.

PV ownership for local files comes from the generated path and expected CA shape. PV ownership for System trust comes from an exact local CA fingerprint match, or from a stale PV-looking certificate with the expected subject when reporting drift. PV should not treat arbitrary third-party roots as PV-owned.

## Error Handling

Domain helpers should expose typed errors where callers need to distinguish behavior. Avoid parsing user-facing strings to distinguish malformed PEM, malformed X.509, key/certificate mismatch, invalid CA shape, unreadable files, keychain inspection failure, current trust, stale trust, denied trust, and missing trust.

Use `anyhow` only at test and top-level orchestration boundaries. CLI command handlers can convert typed errors into user-facing messages through the existing `ExecuteError` path.

Command output should avoid unsafe manual instructions, including raw `sudo`, `security`, or `openssl` snippets. Future-work comments may mark later privileged mutation hooks for PR 13/setup, but they should name the deferred action precisely.

## Testing

Prefer integration-style tests in touched crates and snapshots for user-facing command output.

State tests should cover:

- derived CA certificate path;
- derived CA private-key path;
- layout still creates the certificates directory with user-only permissions.

macOS tests should cover:

- generated CA certificate and key parse successfully;
- generated certificate has the expected PV CA shape;
- generated key matches the generated certificate;
- local CA inspection reports missing, current, repair-required, mismatch, and unreadable states;
- keychain trust classification reports current, not trusted, stale, denied, unknown, and unreadable states using an injected inspector.

CLI integration tests should cover:

- `ca:trust` writes local CA files under injected `HOME`, prints deferred privileged-trust guidance, and exits non-zero;
- `ca:trust` reuses an existing current local CA pair;
- `ca:trust` repairs malformed or mismatched local CA files with a new pair;
- `ca:status` reports local and System trust states without creating files;
- `ca:untrust` leaves local CA files in place and reports deferred privileged removal when trust remains;
- command output does not include unsafe `sudo`, `security`, or `openssl` command snippets.

Keychain tests should use an injected inspector rather than the real System keychain. Certificate generation tests may normalize nondeterministic fields such as serial numbers, validity timestamps, and fingerprints before snapshotting.

Verification should use focused commands first, for example specific `cargo nextest run -E 'test(...)'` invocations and focused `cargo insta test --accept --test-runner nextest -- ...` runs for snapshots. Before opening the PR, run the full workspace tests plus formatting, clippy, dependency, snapshot, and diff hygiene checks.

## Non-Goals

The following are intentionally out of scope for PR 12:

- importing PV's local CA into the System keychain;
- changing System keychain trust settings;
- deleting certificates from the System keychain;
- running `sudo`;
- invoking the `security` CLI;
- invoking `openssl`;
- generating Project leaf certificates;
- configuring Gateway/Caddy/FrankenPHP certificate usage;
- implementing `pv setup` or `pv uninstall`;
- deleting local CA files through `ca:untrust`;
- `pv uninstall --prune` local CA deletion;
- LaunchAgent work;
- daemon health integration for CA trust drift;
- whole-system `pv status` or `pv doctor`;
- DNS resolver or `pf` redirect changes;
- JSON output for `pv ca:*`.
