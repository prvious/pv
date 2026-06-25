# PHP Extension Opt-Ins Design

## Summary

PV will support Project-level PHP extension opt-ins without named profiles or local extension compilation. The default PHP runtime remains lean and Laravel-practical. Projects that need optional extensions list them under `php.extensions`; PV loads only bundled optional modules that are available in the installed PHP artifact and ignores unsupported names with a non-blocking warning.

Optional modules are bundled in PV's PHP/FrankenPHP track artifacts as shared modules, but they are disabled by default. This keeps the first implementation inside the existing PHP artifact lifecycle while preserving the existing boundary: users cannot load arbitrary `.so` files, run `phpize`, or install PECL extensions locally through PV.

## Goals

- Keep existing scalar PHP config valid.
- Add object-form PHP config with optional `version` and `extensions` keys.
- Allow `php.extensions` to request extension names without named presets or profiles.
- Keep default runtime extensions lean but still practical for Laravel apps.
- Bundle the first optional catalog inside PHP/FrankenPHP artifacts as disabled shared modules.
- Load optional modules through PV-generated runtime `conf.d` overlays.
- Group Project-serving FrankenPHP workers by PHP track plus loaded extension set.
- Keep standalone PHP, Composer-through-PHP, and browser execution aligned for a Project.
- Treat unsupported extension names as ignored requests, not invalid Project config.
- Surface ignored names through status/list/log diagnostics so typos are visible.

## Non-Goals

- Do not add named extension profiles, presets, or profile inheritance.
- Do not infer PHP extensions from `composer.json`.
- Do not support arbitrary user-provided shared modules.
- Do not support local PECL, `phpize`, `php-config`, or per-machine extension builds.
- Do not add broad Project-level custom PHP ini settings.
- Do not split optional extensions into separate Managed Resource artifacts in the first version.
- Do not build and ship every extension supported by StaticPHP v3.
- Do not add extensions outside the first curated optional catalog until user demand justifies them.

## Project Config

The current scalar form remains valid:

```yaml
php: 8.4
```

The object form accepts the same version values plus an extension list:

```yaml
php:
  version: 8.4
  extensions:
    - redis
    - xdebug
```

The version may be omitted. In that case PV resolves the PHP track through the same default flow used when `php` is absent:

```yaml
php:
  extensions:
    - xdebug
```

An empty extension list is valid and means no optional extensions:

```yaml
php:
  version: 8.4
  extensions: []
```

`extensions` must be a YAML array of strings. Invalid shapes remain Project config errors because PV cannot interpret them safely. Extension support is not a config validity rule: unsupported strings are accepted, ignored at runtime, and reported as warnings.

The parser must normalize both scalar and object forms into one internal model:

```text
PhpConfig {
  version: Option<PhpTrackSelector>,
  requested_extensions: Vec<String>,
}
```

Duplicate extension names must not create distinct runtimes. The runtime resolver deduplicates requested names after parsing.

## Default And Optional Extensions

The default loaded extension set is Laravel-practical but avoids app-specific service drivers and debugging tools:

```text
bcmath
ctype
curl
dom
fileinfo
filter
hash
iconv
intl
json
libxml
mbstring
openssl
pcntl
pcre
pdo
pdo_mysql
pdo_pgsql
pdo_sqlite
phar
posix
session
simplexml
sodium
sqlite3
tokenizer
xml
xmlreader
xmlwriter
zip
zlib
```

The initial optional catalog is:

```text
redis
sqlsrv
pdo_sqlsrv
xdebug
apcu
pcov
imagick
mongodb
yaml
```

This moves `redis`, `sqlsrv`, and `pdo_sqlsrv` out of the current always-loaded set. `xdebug`, `apcu`, `pcov`, `imagick`, `mongodb`, and `yaml` are new opt-in candidates.

Future extensions should be added only when users ask for them and PV can build, smoke-test, license, and support them across the intended PHP track/platform matrix.

## Artifact Packaging

PV continues publishing paired PHP and FrankenPHP artifacts per PHP track. Each track artifact includes:

- the standalone `php` binary,
- the matched `frankenphp` binary for the same PHP patch version,
- default PHP runtime files,
- default compiled/static extensions,
- bundled optional shared modules for the curated catalog.

Bundled optional modules are disabled by default. PV enables them by writing generated `.ini` files into a runtime-specific `conf.d` overlay. Normal extensions use `extension=...`; Zend extensions such as Xdebug use `zend_extension=...`.

The PHP artifact manifest or artifact metadata must expose the optional catalog for each artifact, including at least:

- extension name,
- load kind: `extension` or `zend_extension`,
- module path relative to the active artifact root,
- whether the module is available for the current platform/artifact.

Keeping the catalog in artifact metadata lets PV add optional bundled modules in future PHP artifact releases without requiring a PV app release, as long as the installed PV version understands the manifest schema.

If a PHP artifact does not advertise optional extension metadata, PV treats it as having no optional bundled extensions. Projects still serve with the default runtime and ignored-extension warnings.

## Runtime Resolution

PV resolves each linked Project to a PHP runtime identity:

```text
PHP track + sorted available extension names
```

Examples:

```text
8.4                  -> default 8.4 runtime
8.4 + redis          -> 8.4 runtime with redis
8.4 + redis + xdebug -> 8.4 runtime with redis and xdebug
8.5 + redis          -> 8.5 runtime with redis
```

The requested extension order in YAML does not affect runtime identity. These are equivalent:

```yaml
extensions: [redis, xdebug]
```

```yaml
extensions: [xdebug, redis]
```

Unsupported requested extensions are excluded from the runtime identity. A Project that requests `redis` and `fake_extension` uses the same runtime as a Project that requests only `redis`, with `fake_extension` reported as ignored.

## FrankenPHP Workers

Project-serving workers are grouped by PHP runtime identity, not by PHP track alone. Projects share a worker only when both the track and the loaded optional extension set match.

This preserves extension startup semantics. PHP extensions are loaded when the PHP runtime starts, so PV cannot safely serve Projects with different extension sets from the same FrankenPHP worker.

Worker config, pid files, runtime metadata, log paths, observed runtime subjects, and port ownership need to use the runtime identity rather than only the PHP track. Implementations may use a readable slug for short identities and a stable hash if the identity becomes too long for paths or subjects.

When a Project changes only its optional extension set, PV reassigns it to the matching worker, starts that worker if needed, and stops the old worker if no Projects remain. Unrelated PHP runtimes are not touched.

## CLI And Composer

The `php` shim must resolve the current Project's PHP runtime identity when executed inside a linked Project. It should set `PHPRC` and `PHP_INI_SCAN_DIR` so the standalone CLI process sees the same default ini plus generated optional-extension overlay as the Project's browser runtime.

The `composer` shim already runs through PV's PHP selection path. It should inherit the same Project PHP runtime identity and extension overlay as direct `php` commands. This avoids CLI/browser drift for Composer scripts that depend on loaded extensions.

Outside a linked Project, PHP and Composer use the global/default PHP track with no Project-level optional extensions.

## Ini Overlay

PV keeps track-level defaults under the existing mutable track defaults directory:

```text
~/.pv/resources/php/<track>/etc/php.ini
~/.pv/resources/php/<track>/etc/conf.d/
```

For extension opt-ins, PV adds generated runtime overlays under PV-owned config storage, for example:

```text
~/.pv/config/php-runtimes/<runtime-key>/conf.d/
```

Runtime processes use both scan directories:

```text
PHP_INI_SCAN_DIR=<track-default-conf-d>:<runtime-extension-conf-d>
```

Generated extension ini files are owned by PV and replaced wholesale during reconciliation. Users must not edit them. User-editable track defaults remain separate.

The overlay includes only available optional extensions requested by at least one Project using that runtime. It does not include unsupported names.

## Unsupported Extensions

Unsupported extension names are not Project config errors. PV must:

1. Parse and accept the config when `extensions` is an array of strings.
2. Resolve the requested names against the artifact's optional catalog.
3. Load available names.
4. Ignore unavailable names.
5. Report ignored names in diagnostics.

Example:

```yaml
php:
  extensions:
    - redis
    - fake_extension
```

If `redis` is available, the Project runs with `redis`; `fake_extension` is ignored. The Project should continue serving.

Ignored-extension reporting must appear in at least one user-visible place such as `pv list`, `pv status`, or structured Project diagnostics. It is non-blocking and does not mark the Project config invalid.

## State And Manifest Impact

State that currently stores only a desired PHP track for each Project needs to preserve enough information to recover the last valid runtime when the config later becomes invalid. The stored runtime must include:

- resolved concrete PHP track,
- requested extension names,
- available loaded extension names,
- ignored extension names.

Runtime observed subjects and port owners should identify workers by runtime identity. Existing track-only subjects such as `php_worker:8.4` need a compatible replacement that can distinguish `8.4` from `8.4+redis`.

Manifest parsing should grow optional PHP extension metadata. Older manifests or artifacts without the metadata are interpreted as supporting no optional extensions. New artifacts that rely on extension metadata should raise `minimum_pv_version` when needed so old PV versions do not incorrectly claim support for a feature they cannot apply.

## Artifact Recipe Impact

The PHP recipe must split extension settings into default loaded extensions and optional bundled shared extensions.

The recipe build must:

- build default extensions into the PHP/FrankenPHP runtime as today,
- build optional catalog entries as shared modules when StaticPHP supports that mode,
- package optional modules in a stable artifact-relative location,
- emit metadata describing each optional module's load kind and path,
- smoke-test the default runtime without optional modules loaded,
- smoke-test each optional module by loading it through a generated ini overlay for both standalone PHP and FrankenPHP where supported.

StaticPHP v3 extension caveats still apply. Extensions that are not compatible with PV's macOS/ZTS/FrankenPHP requirements should not enter the PV optional catalog until proven.

## Error Handling

Invalid PHP config shape remains blocking:

- `php` object with unknown shape,
- `extensions` not an array,
- extension entries that are not strings.

Unsupported extension names are non-blocking warnings.

If an extension is advertised by artifact metadata but the shared module file is missing, that is an artifact/runtime error. PV must fail the affected runtime readiness or mark it degraded rather than silently serving without an advertised extension.

If an optional extension causes the worker to fail readiness, the failure is scoped to that PHP runtime identity. Other PHP runtimes continue serving.

## Testing

Prefer integration tests and snapshots where practical.

Config tests should cover:

- scalar `php` remains valid,
- object `php.version` resolves like scalar `php`,
- object `php.extensions` with omitted version resolves through the default track,
- `extensions: []` is valid,
- invalid extension shapes are config errors,
- unsupported extension strings are accepted.

Runtime planning tests should cover:

- Projects with the same track and same extension set share a worker,
- extension order does not create different workers,
- unsupported extension names do not create distinct workers,
- Projects with different available extension sets use different workers,
- invalid Project config preserves the last valid runtime assignment.

Shim tests should cover:

- PHP shim environment includes the runtime overlay inside a linked Project,
- Composer inherits the same PHP runtime overlay,
- outside linked Projects, shims use the global/default track without Project extensions.

Artifact/release tests should cover:

- recipe metadata distinguishes default loaded extensions from optional bundled extensions,
- generated manifests expose optional PHP extension metadata,
- archive validation requires advertised optional module files,
- smoke tests verify default runtimes do not load optional modules,
- smoke tests verify each optional module can load through the generated overlay.
