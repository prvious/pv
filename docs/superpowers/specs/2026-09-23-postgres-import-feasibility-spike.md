# PostgreSQL import feasibility spike

Status: partial spike for #351. Multi-database `pg_dumpall` support is required in this PR. No dump from this corpus is safe to execute against a PV-managed cluster yet.

## Reproduce the corpus

`sh it/fixtures/postgres-import/generate.sh` uses disposable `postgres:17.11-alpine` and `postgres:18.6-alpine` containers. The checked-in source creates three user databases (including quoted `"Mixed-Name"`), production ownership and grants, role membership, a tablespace, a large object, a database setting, `pgcrypto`, `SECURITY DEFINER`, a publication, an event trigger in `postgres`, a dollar-quoted function containing database names and a semicolon, non-ASCII and backslash data. For each version the script saves plain, `--create`, custom, tar, directory, and `pg_dumpall` outputs. It uses a fixed `--restrict-key` **only to keep fixtures readable**; importer security must never depend on a predictable key. `hostile.sql` is a hand-written rejection input.

The generated `pg_dumpall` fixture omits role password hashes but retains role commands, so it must not be restored without filtering.

## Observations

| Question | Result on both 17.11 and 18.6 |
| --- | --- |
| Can `pg_restore --create --dbname=postgres` remap a database? | No. It recreated `production` in an isolated cluster; the designated empty target stayed empty. PostgreSQL documents that `--dbname` is only the initial connection with `--create`. |
| Can archives be rendered before connecting? | Yes. Matching `pg_restore --file=... --create` rendered custom, tar, and directory fixtures. Each rendered script contained `CREATE DATABASE`, `\\connect`, and `COPY`. |
| Can the archive table of contents prove safety? | No. It identifies objects but omits their full SQL bodies. PostgreSQL warns that restore can execute source-chosen code even from partial dumps. |
| Can PostgreSQL's parser read the generated SQL? | `pg_query` 6.2.0 parsed all six SQL fixtures after an outer reader removed `psql` command lines and `COPY` data. The current checked-in corpus has 37 statements for plain, 62 for `--create`, and 162 for `pg_dumpall`, per version. This is syntax coverage, not permission to execute. Its build required libclang inside the Rust container. |
| Can the pure Rust comparison parser read them? | `sqlparser` 0.63.0 parsed the plain fixtures (20 statements), but rejected generated `CREATE DATABASE ... WITH ...` in `--create` and `pg_dumpall` at `WITH`. |
| Does a restricted SQL role contain mistakes? | Partly. A role without `SUPERUSER` or `CREATEROLE` could not create a role. It could not connect to a database only after `CONNECT` was revoked from `PUBLIC`. With PostgreSQL's default grants, it could connect to another database. A `psql \\!` command still ran on the client regardless of database role. |
| Can connection routing be restricted without changing other database grants? | Yes in an isolated 18.6 container. A first-match `pg_hba.conf` rule allowed the import role to connect to its mapped target, and a following `reject` rule denied another database despite its default `PUBLIC CONNECT` grant. This proves the mechanism, not safe configuration ownership or crash recovery in PV. |
| Are generated `\\connect` lines always a bare database name? | No. The quoted database produced `\\connect -reuse-previous=on "dbname='Mixed-Name'"` on both versions. Routing analysis must parse this generated form and reject other connection parameters. |
| Are other `psql` commands needed for quoted names? | Yes. Both `pg_dumpall` outputs put `\\encoding SQL_ASCII` immediately before the quoted connection. The preflight must account for that encoding transition rather than accepting arbitrary `\\encoding` values. |
| Can the parser identify a database name without regenerating SQL? | In a focused probe, `pg_query` parsed `CREATE DATABASE "Mixed-Name"` as database `Mixed-Name`; its scanner returned the exact byte span of the quoted identifier. This supports targeted patches, but each accepted database-container statement still needs verified AST and token matching. |
| Can preflight recognize `SECURITY DEFINER` structurally? | Yes. A focused `pg_query` AST test reports the function's `security` option as boolean `true`, rather than relying on a text search that could match a comment or body string. |

## Recommendation and remaining proof

`sh it/fixtures/postgres-import/isolation.sh` now repeats the role/HBA probe on both supported fixed versions. It verifies target connection succeeds, unrelated Project connection is rejected, role creation fails, an imported table survives ownership reassignment and role removal, and the original HBA file is restored. The user approved this isolation model and an explicit `pg_dumpall` plan that skips cluster globals and the `postgres` database unless it is mapped. They also chose to reject `SECURITY DEFINER` functions and procedures rather than change their execution privileges when ownership moves to `pv_root`. The production implementation still needs crash recovery and must verify the reload and active rules before starting a restore.

Use archive-to-SQL rendering and one PostgreSQL-specific preflight path before any live restore. The SQL analyzer should use PostgreSQL parser nodes for statement policy and verified source spans for identifier patches; a bounded outer reader still has to frame `psql` commands and `COPY` payloads. The pure Rust parser is not a drop-in choice for actual 17/18 `--create` and `pg_dumpall` output. Direct `pg_restore` should not be the first execution path because a TOC-only check cannot establish the issue's complete-preflight boundary.

This is not yet a go decision for execution. Fixture tests now prove that the reader frames quoted identifiers, nested comments, dollar quotes, `COPY` terminators, CRLF, large rows, and hostile meta-commands without treating data as commands. Define a non-lossy `postgres` policy and explicit skip/reject policy for roles, tablespaces, database settings, ownership, ACLs, and the other global classes in #351. Test actual execution with a role and connection policy that protects unrelated Project databases under PV's default grants; a restricted role alone is insufficient. Keep the source snapshot and transformed output on owner-only storage, and execute neither until the complete plan is accepted.

The `postgres-import` crate now contains a read-only, bounded-memory framing and syntax check with fixture snapshots for both fixed versions. It recognizes plain, custom, and tar regular-file headers; directory input still needs a path-level policy. It does not yet classify all SQL effects, map targets, transform scripts, or authorize execution.

The reader deliberately rejects unsupported `psql` connection strings and non-default `COPY` modes before either can affect framing. SQL statements currently have an 8 MiB limit; `COPY` rows stream without that limit. It rejects non-UTF-8 SQL, including SQL_ASCII scripts that actually contain invalid UTF-8 bytes. These are explicit conservative input limits, not successful-import coverage.

The recipe prerequisite is separate: update 17.10/18.4 to security-fixed 17.11/18.6 and package `pg_restore`. The source SHA-256 values in the recipe were computed from official archives in a disposable Docker container. Native macOS artifact smoke remains necessary because Linux compilation does not establish the Mach-O packaging and code-signing behavior.
