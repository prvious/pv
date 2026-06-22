# Use fixed-extension PHP and FrankenPHP artifacts

Status: Superseded by [ADR 0014](0014-project-level-php-extension-opt-ins.md).

PV v1 will avoid PHP extension management and instead distribute prebuilt macOS PHP and FrankenPHP artifacts with a common extension set baked in. Standalone PHP and FrankenPHP are built as single-binary/static-style artifacts with fixed compiled-in extensions, no Homebrew dependency, and no support for dynamic extension loading, `phpize`, or PECL-installed extensions. This keeps local setup predictable and avoids building or configuring extensions per machine or per Project, at the cost of making unsupported extension needs outside v1 scope.
