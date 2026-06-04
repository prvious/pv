use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use macos::{
    PfConfReference, PfRedirectConfig, ResolverConfig, inspect_pf_anchor_file,
    inspect_pf_conf_reference, inspect_resolver_file, loopback_tcp_listener_ports,
};
use state::fs;

#[test]
fn resolver_config_renders_pv_owned_test_resolver_file() {
    let config = ResolverConfig::new(35353);
    let rendered = config.render();

    assert_debug_snapshot!(&rendered);
    assert_eq!(ResolverConfig::parse(&rendered), Some(config));
}

#[test]
fn resolver_file_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current");
    let stale_path = tempdir.path().join("stale");
    let wrong_nameserver_path = tempdir.path().join("wrong-nameserver");
    let mixed_nameservers_path = tempdir.path().join("mixed-nameservers");
    let conflict_path = tempdir.path().join("conflict");
    let unreadable_path = tempdir.path().join("directory");
    let expected = ResolverConfig::new(35353);

    fs::write_sensitive_file(&current_path, &expected.render())?;
    fs::write_sensitive_file(&stale_path, &ResolverConfig::new(45000).render())?;
    fs::write_sensitive_file(
        &wrong_nameserver_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(
        &mixed_nameservers_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 127.0.0.1\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(&conflict_path, "nameserver 127.0.0.1\nport 35353\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_resolver_file(&tempdir.path().join("missing"), Some(&expected)),
        inspect_resolver_file(&current_path, Some(&expected)),
        inspect_resolver_file(&stale_path, Some(&expected)),
        inspect_resolver_file(&wrong_nameserver_path, Some(&expected)),
        inspect_resolver_file(&mixed_nameservers_path, Some(&expected)),
        inspect_resolver_file(&conflict_path, Some(&expected)),
        inspect_resolver_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_config_renders_pv_owned_anchor_and_pf_conf_reference() {
    let config = PfRedirectConfig::new(48080, 48443);
    let anchor = config.render_anchor();
    let reference = PfConfReference.render();

    assert_eq!(PfRedirectConfig::parse_anchor(&anchor), Some(config));
    assert_eq!(
        PfConfReference::parse_block(&reference),
        Some(PfConfReference)
    );
    assert_debug_snapshot!((anchor, reference));
}

#[test]
fn pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-anchor");
    let stale_path = tempdir.path().join("stale-anchor");
    let extra_active_rule_path = tempdir.path().join("extra-active-rule-anchor");
    let malformed_path = tempdir.path().join("malformed-anchor");
    let conflict_path = tempdir.path().join("conflict-anchor");
    let unreadable_path = tempdir.path().join("anchor-directory");
    let expected = PfRedirectConfig::new(48080, 48443);

    fs::write_sensitive_file(&current_path, &expected.render_anchor())?;
    fs::write_sensitive_file(
        &stale_path,
        &PfRedirectConfig::new(45000, 45001).render_anchor(),
    )?;
    fs::write_sensitive_file(
        &extra_active_rule_path,
        &format!("{}pass in all\n", expected.render_anchor()),
    )?;
    fs::write_sensitive_file(&malformed_path, "# Managed by PV\npass in all\n")?;
    fs::write_sensitive_file(
        &conflict_path,
        "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\n",
    )?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_anchor_file(&tempdir.path().join("missing-anchor"), Some(&expected)),
        inspect_pf_anchor_file(&current_path, Some(&expected)),
        inspect_pf_anchor_file(&stale_path, Some(&expected)),
        inspect_pf_anchor_file(&extra_active_rule_path, Some(&expected)),
        inspect_pf_anchor_file(&malformed_path, Some(&expected)),
        inspect_pf_anchor_file(&conflict_path, Some(&expected)),
        inspect_pf_anchor_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()>
{
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-pf-conf");
    let stale_path = tempdir.path().join("stale-pf-conf");
    let conflict_path = tempdir.path().join("conflict-pf-conf");
    let active_anchor_conflict_path = tempdir.path().join("active-anchor-conflict-pf-conf");
    let active_load_conflict_path = tempdir.path().join("active-load-conflict-pf-conf");
    let commented_reference_path = tempdir.path().join("commented-reference-pf-conf");
    let prose_reference_path = tempdir.path().join("prose-reference-pf-conf");
    let unrelated_path = tempdir.path().join("unrelated-pf-conf");
    let unreadable_path = tempdir.path().join("pf-conf-directory");
    let expected = PfConfReference;

    fs::write_sensitive_file(
        &current_path,
        &format!(
            "set block-policy drop\n{}\npass out all\n",
            expected.render()
        ),
    )?;
    fs::write_sensitive_file(
        &stale_path,
        "# Managed by PV\nanchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/tmp/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &conflict_path,
        "anchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(&active_anchor_conflict_path, "anchor \"com.prvious.pv\"\n")?;
    fs::write_sensitive_file(
        &active_load_conflict_path,
        "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &commented_reference_path,
        "# anchor \"com.prvious.pv\"\n# load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &prose_reference_path,
        "This note mentions com.prvious.pv and /etc/pf.anchors/com.prvious.pv.\n",
    )?;
    fs::write_sensitive_file(&unrelated_path, "set block-policy drop\npass out all\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_conf_reference(&tempdir.path().join("missing-pf-conf"), Some(&expected)),
        inspect_pf_conf_reference(&current_path, Some(&expected)),
        inspect_pf_conf_reference(&stale_path, Some(&expected)),
        inspect_pf_conf_reference(&conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&active_anchor_conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&active_load_conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&commented_reference_path, Some(&expected)),
        inspect_pf_conf_reference(&prose_reference_path, Some(&expected)),
        inspect_pf_conf_reference(&unrelated_path, Some(&expected)),
        inspect_pf_conf_reference(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_loopback_tcp_listener_ports_include_ipv4_wildcard_listener() -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let port = listener.local_addr()?.port();
    let ports = loopback_tcp_listener_ports()?;
    let detection = vec![("ipv4 wildcard listener detected", ports.contains(&port))];

    assert_debug_snapshot!(detection);

    Ok(())
}

#[test]
fn pf_loopback_tcp_listener_ports_include_ipv6_loopback_and_wildcard_listeners() -> Result<()> {
    let loopback_listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0))?;
    let wildcard_listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0))?;
    let loopback_port = loopback_listener.local_addr()?.port();
    let wildcard_port = wildcard_listener.local_addr()?.port();
    let ports = loopback_tcp_listener_ports()?;
    let detections = vec![
        (
            "ipv6 loopback listener detected",
            ports.contains(&loopback_port),
        ),
        (
            "ipv6 wildcard listener detected",
            ports.contains(&wildcard_port),
        ),
    ];

    assert_debug_snapshot!(detections);

    Ok(())
}

fn normalize_state_debug(state: &impl Debug, tempdir_path: &str) -> String {
    let state_debug = format!("{state:?}").replace(tempdir_path, "[tempdir]");

    if let Some((prefix, _message)) = state_debug.split_once(", message: ")
        && prefix.starts_with("Unreadable ")
    {
        return format!("{prefix}, message: \"<read error>\" }}");
    }

    state_debug
}
