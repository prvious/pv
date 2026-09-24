use std::io::{Cursor, Read};

use insta::assert_debug_snapshot;
use pg_query::protobuf::node::Node;
use postgres_import::{detect_regular_dump_format, inspect_plain_dump};

const PLAIN_17: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/17.11/plain.sql");
const CREATE_17: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/17.11/create.sql");
const ALL_17: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/17.11/all.sql");
const PLAIN_18: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/18.6/plain.sql");
const CREATE_18: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/18.6/create.sql");
const ALL_18: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/18.6/all.sql");
const CUSTOM_17: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/17.11/custom.dump");
const TAR_17: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/17.11/tar.dump");
const CUSTOM_18: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/18.6/custom.dump");
const TAR_18: &[u8] = include_bytes!("../../../it/fixtures/postgres-import/18.6/tar.dump");

#[test]
fn regular_file_format_is_detected_from_contents() -> anyhow::Result<()> {
    let results = [
        ("17.11 plain", PLAIN_17),
        ("17.11 custom", CUSTOM_17),
        ("17.11 tar", TAR_17),
        ("18.6 plain", PLAIN_18),
        ("18.6 custom", CUSTOM_18),
        ("18.6 tar", TAR_18),
    ]
    .into_iter()
    .map(|(name, fixture)| Ok((name, detect_regular_dump_format(Cursor::new(fixture))?)))
    .collect::<anyhow::Result<Vec<_>>>()?;

    assert_debug_snapshot!(results);
    Ok(())
}

#[test]
fn generated_dumps_frame_without_treating_data_as_commands() -> anyhow::Result<()> {
    let results = [
        ("17.11 plain", PLAIN_17),
        ("17.11 create", CREATE_17),
        ("17.11 all", ALL_17),
        ("18.6 plain", PLAIN_18),
        ("18.6 create", CREATE_18),
        ("18.6 all", ALL_18),
    ]
    .into_iter()
    .map(|(name, fixture)| Ok((name, inspect_plain_dump(Cursor::new(fixture))?)))
    .collect::<anyhow::Result<Vec<_>>>()?;

    assert_debug_snapshot!(results);
    Ok(())
}

#[test]
fn hostile_meta_commands_fail_before_execution() {
    let cases = [
        "\\! touch /tmp/should-not-run\n",
        "\\include /tmp/should-not-run\n",
        "\\include_relative other.sql\n",
        "\\copy public.proof FROM PROGRAM 'touch /tmp/should-not-run'\n",
        "\\connect admin host=other\n",
        "\\connect postgresql://other/db\n",
        "\\connect -reuse-previous=on \"dbname='admin' host=other\"\n",
    ];
    let errors = cases
        .into_iter()
        .map(|case| {
            inspect_plain_dump(Cursor::new(case))
                .err()
                .map(|error| error.to_string())
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(errors);
}

#[test]
fn copy_data_requires_its_exact_terminator() {
    let input = b"CREATE TABLE proof (value text);\nCOPY proof (value) FROM stdin;\n\\\\.\n";
    let result = inspect_plain_dump(Cursor::new(input));
    assert_debug_snapshot!(result.err().map(|error| error.to_string()));
}

#[test]
fn nested_comments_dollar_quotes_and_crlf_are_framed() -> anyhow::Result<()> {
    let input = b"/* outer /* inner ; */ still comment ; */\r\nCREATE FUNCTION f() RETURNS text LANGUAGE sql AS $body$ SELECT 'admin; \\connect postgres'; $body$;\r\nCREATE TABLE proof (value text);\r\nCOPY proof (value) FROM stdin;\r\n\\\\.\r\n\\.\r\n";
    assert_debug_snapshot!(inspect_plain_dump(Cursor::new(input))?);
    Ok(())
}

#[test]
fn very_large_copy_row_does_not_hit_statement_limit() -> anyhow::Result<()> {
    let header = Cursor::new(b"CREATE TABLE proof (value text);\nCOPY proof (value) FROM stdin;\n");
    let row = std::io::repeat(b'x').take(9 * 1024 * 1024);
    let end = Cursor::new(b"\n\\.\n");
    assert_debug_snapshot!(inspect_plain_dump(header.chain(row).chain(end))?);
    Ok(())
}

#[test]
fn server_side_copy_program_is_rejected() {
    let input = b"COPY proof FROM PROGRAM 'touch /tmp/should-not-run';\n";
    assert_debug_snapshot!(
        inspect_plain_dump(Cursor::new(input))
            .err()
            .map(|error| error.to_string())
    );
}

#[test]
fn copy_options_that_change_data_framing_are_rejected() {
    let input = b"COPY public.proof FROM STDIN WITH (FORMAT binary);\n";
    assert_debug_snapshot!(
        inspect_plain_dump(Cursor::new(input))
            .err()
            .map(|error| error.to_string())
    );
}

#[test]
fn restriction_key_must_match_and_close() {
    let cases = [
        "\\restrict key\n\\unrestrict other\n",
        "\\restrict key\n",
        "\\unrestrict key\n",
        "\\restrict key\n\\restrict key\n",
    ];
    let errors = cases
        .into_iter()
        .map(|case| {
            inspect_plain_dump(Cursor::new(case))
                .err()
                .map(|error| error.to_string())
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(errors);
}

#[test]
fn postgres_scanner_exposes_database_identifier_byte_spans() -> anyhow::Result<()> {
    let sql = "CREATE DATABASE \"Mixed-Name\" WITH TEMPLATE = template0;";
    let parsed = pg_query::parse(sql)?;
    let tokens = pg_query::scan(sql)?
        .tokens
        .into_iter()
        .map(|token| {
            let start = usize::try_from(token.start)?;
            let end = usize::try_from(token.end)?;
            Ok((start, end, &sql[start..end]))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let database_name = parsed
        .protobuf
        .stmts
        .first()
        .and_then(|statement| statement.stmt.as_ref())
        .and_then(|node| node.node.as_ref())
        .and_then(|node| match node {
            Node::CreatedbStmt(statement) => Some(statement.dbname.as_str()),
            _ => None,
        });
    assert_debug_snapshot!((database_name, tokens));
    Ok(())
}

#[test]
fn postgres_parser_marks_security_definer_functions() -> anyhow::Result<()> {
    let sql = "CREATE FUNCTION public.proof() RETURNS integer LANGUAGE sql SECURITY DEFINER AS $$ SELECT 1 $$;";
    let parsed = pg_query::parse(sql)?;
    let options = parsed
        .protobuf
        .stmts
        .first()
        .and_then(|statement| statement.stmt.as_ref())
        .and_then(|node| node.node.as_ref())
        .and_then(|node| match node {
            Node::CreateFunctionStmt(statement) => Some(&statement.options),
            _ => None,
        })
        .into_iter()
        .flatten()
        .filter_map(|option| match option.node.as_ref() {
            Some(Node::DefElem(option)) => Some((
                option.defname.as_str(),
                option
                    .arg
                    .as_ref()
                    .and_then(|arg| arg.node.as_ref())
                    .and_then(|arg| match arg {
                        Node::Boolean(value) => Some(value.boolval),
                        _ => None,
                    }),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(options);
    Ok(())
}
