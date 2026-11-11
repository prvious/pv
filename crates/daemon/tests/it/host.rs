use anyhow::Result;
use daemon::{DnsRepair, GatewayListenerHolder, HostIntegration};
use platform::GatewayListenerSource;

/// The daemon's host for a test: two test ports for the Gateway instead of ports 80 and 443, and
/// no DNS repair, which would reach the machine's own helper.
/// Every daemon a test starts shares one, so a Gateway one daemon started is probed where it
/// listens.
pub(crate) fn test_host() -> Result<HostIntegration> {
    let [http_port, https_port] = pv_fake::ports()?;

    Ok(HostIntegration {
        gateway_listeners: GatewayListenerSource::Bind {
            http_port,
            https_port,
        },
        dns_repair: DnsRepair::Off,
    })
}

/// The Gateway's listeners for a test that reconciles without a daemon.
pub(crate) fn test_gateway_listeners() -> Result<GatewayListenerHolder> {
    Ok(GatewayListenerHolder::new(test_host()?.gateway_listeners))
}
