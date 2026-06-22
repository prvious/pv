# Support Project-level PHP extension opt-ins

PV will support Project-level PHP extension opt-ins through `php.extensions` while keeping PHP and FrankenPHP artifacts prebuilt and PV-owned. Existing scalar PHP config remains valid, and object-form config may specify `version` and `extensions`.

Optional extensions are not named profiles or presets. A Project lists the extensions it wants directly. PV loads only bundled optional modules available in the installed PHP artifact, ignores unsupported names, and surfaces ignored names as non-blocking diagnostics.

PV will keep the default loaded PHP extension set lean but Laravel-practical. The initial optional catalog is `redis`, `sqlsrv`, `pdo_sqlsrv`, `xdebug`, `apcu`, `pcov`, `imagick`, `mongodb`, and `yaml`. Future extensions should be added only when user demand justifies the build, smoke-test, license, and support burden.

Optional modules will be bundled in the existing PHP/FrankenPHP track artifacts as disabled shared modules. PV will enable them by generating runtime-specific ini overlays rather than by installing separate extension artifacts in the first implementation. This makes track artifacts somewhat larger but avoids separate extension artifact resolution, ABI matching, install/update transactions, and manifest nesting until the catalog grows enough to justify that complexity.

Project-serving FrankenPHP workers are grouped by PHP runtime identity: resolved PHP track plus sorted available optional extension names. Standalone PHP, Composer-through-PHP, and browser execution for a Project must use the same resolved runtime identity so CLI and browser behavior do not drift.

PV still does not support arbitrary user-provided `.so` files, local PECL installs, `phpize`, `php-config`, custom per-Project PHP ini settings, or building every extension StaticPHP supports.
