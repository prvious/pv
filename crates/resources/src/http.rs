use std::io::{Read, Write};
use std::time::Duration;

use crate::{ResourcesError, Result};

const DOWNLOAD_BUFFER_SIZE: usize = 8192;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MANIFEST_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const MANIFEST_BODY_TIMEOUT: Duration = Duration::from_secs(30);
const ARTIFACT_BODY_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub trait ResourceHttpClient {
    /// Fetches UTF-8 text from `url`.
    ///
    /// Implementations should return [`ResourcesError::HttpRequestFailed`] for
    /// transport or response-body read failures and
    /// [`ResourcesError::HttpStatusFailed`] for HTTP status responses.
    fn get_text(&self, url: &str) -> Result<String>;

    /// Streams bytes from `url` into `writer`.
    ///
    /// Implementations should return [`ResourcesError::HttpRequestFailed`] for
    /// transport or response-body read failures and
    /// [`ResourcesError::HttpStatusFailed`] for HTTP status responses.
    /// Destination write failures must use a non-retriable error such as
    /// [`ResourcesError::DownloadWriteFailed`] so the downloader does not retry
    /// local filesystem failures as network failures.
    fn download(&self, url: &str, writer: &mut dyn Write) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct UreqResourceHttpClient {
    text_agent: ureq::Agent,
    download_agent: ureq::Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResourceHttpTimeouts {
    resolve: Option<Duration>,
    connect: Option<Duration>,
    recv_response: Option<Duration>,
    recv_body: Option<Duration>,
}

impl Default for UreqResourceHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqResourceHttpClient {
    pub fn new() -> Self {
        Self::with_timeouts(
            ResourceHttpTimeouts::manifest(),
            ResourceHttpTimeouts::artifact(),
        )
    }

    fn with_timeouts(
        text_timeouts: ResourceHttpTimeouts,
        download_timeouts: ResourceHttpTimeouts,
    ) -> Self {
        Self {
            text_agent: agent_with_timeouts(text_timeouts),
            download_agent: agent_with_timeouts(download_timeouts),
        }
    }

    #[cfg(test)]
    fn with_timeouts_for_test(timeouts: ResourceHttpTimeouts) -> Self {
        Self::with_timeouts(timeouts, timeouts)
    }

    #[cfg(test)]
    fn configured_text_timeouts(&self) -> ureq::config::Timeouts {
        self.text_agent.config().timeouts()
    }

    #[cfg(test)]
    fn configured_download_timeouts(&self) -> ureq::config::Timeouts {
        self.download_agent.config().timeouts()
    }
}

impl ResourceHttpTimeouts {
    fn manifest() -> Self {
        Self {
            resolve: Some(CONNECT_TIMEOUT),
            connect: Some(CONNECT_TIMEOUT),
            recv_response: Some(MANIFEST_RESPONSE_TIMEOUT),
            recv_body: Some(MANIFEST_BODY_TIMEOUT),
        }
    }

    fn artifact() -> Self {
        Self {
            resolve: Some(CONNECT_TIMEOUT),
            connect: Some(CONNECT_TIMEOUT),
            recv_response: Some(MANIFEST_RESPONSE_TIMEOUT),
            recv_body: Some(ARTIFACT_BODY_TIMEOUT),
        }
    }
}

fn agent_with_timeouts(timeouts: ResourceHttpTimeouts) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(timeouts.resolve)
        .timeout_connect(timeouts.connect)
        .timeout_recv_response(timeouts.recv_response)
        .timeout_recv_body(timeouts.recv_body)
        .build()
        .into()
}

impl ResourceHttpClient for UreqResourceHttpClient {
    fn get_text(&self, url: &str) -> Result<String> {
        let mut response = self
            .text_agent
            .get(url)
            .call()
            .map_err(|source| http_error(url, source))?;

        response
            .body_mut()
            .read_to_string()
            .map_err(|source| ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: source.to_string(),
            })
    }

    fn download(&self, url: &str, writer: &mut dyn Write) -> Result<()> {
        let mut response = self
            .download_agent
            .get(url)
            .call()
            .map_err(|source| http_error(url, source))?;
        let mut reader = response.body_mut().as_reader();
        let mut buffer = [0_u8; DOWNLOAD_BUFFER_SIZE];

        loop {
            let read =
                reader
                    .read(&mut buffer)
                    .map_err(|source| ResourcesError::HttpRequestFailed {
                        url: url.to_string(),
                        reason: source.to_string(),
                    })?;
            if read == 0 {
                return Ok(());
            }

            writer.write_all(&buffer[..read]).map_err(|source| {
                ResourcesError::DownloadWriteFailed {
                    url: url.to_string(),
                    reason: source.to_string(),
                }
            })?;
        }
    }
}

fn http_error(url: &str, source: ureq::Error) -> ResourcesError {
    match source {
        ureq::Error::StatusCode(status_code) => ResourcesError::HttpStatusFailed {
            url: url.to_string(),
            status_code,
        },
        error => ResourcesError::HttpRequestFailed {
            url: url.to_string(),
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use anyhow::Result;

    use super::{ResourceHttpTimeouts, UreqResourceHttpClient};
    use crate::{ResourceHttpClient, ResourcesError};

    #[test]
    fn default_client_configures_finite_timeouts() {
        let client = UreqResourceHttpClient::default();
        let text_timeouts = client.configured_text_timeouts();
        let download_timeouts = client.configured_download_timeouts();

        assert_eq!(text_timeouts.resolve, Some(Duration::from_secs(5)));
        assert_eq!(text_timeouts.connect, Some(Duration::from_secs(5)));
        assert_eq!(text_timeouts.recv_response, Some(Duration::from_secs(30)));
        assert_eq!(text_timeouts.recv_body, Some(Duration::from_secs(30)));
        assert_eq!(download_timeouts.resolve, Some(Duration::from_secs(5)));
        assert_eq!(download_timeouts.connect, Some(Duration::from_secs(5)));
        assert_eq!(
            download_timeouts.recv_response,
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            download_timeouts.recv_body,
            Some(Duration::from_secs(30 * 60))
        );
    }

    #[test]
    fn ureq_client_times_out_when_response_headers_stall() -> Result<()> {
        let client = UreqResourceHttpClient::with_timeouts_for_test(ResourceHttpTimeouts {
            resolve: Some(Duration::from_millis(40)),
            connect: Some(Duration::from_millis(40)),
            recv_response: Some(Duration::from_millis(40)),
            recv_body: Some(Duration::from_millis(40)),
        });

        serve_stalled_response(|url| {
            let started = Instant::now();
            let result = client.get_text(&url);

            let Err(ResourcesError::HttpRequestFailed { reason, .. }) = result else {
                return Err(anyhow::anyhow!(
                    "expected timeout request failure, got {result:?}"
                ));
            };
            assert!(started.elapsed() < Duration::from_millis(200));
            assert!(reason.contains("timeout"));

            Ok(())
        })
    }

    fn serve_stalled_response(operation: impl FnOnce(String) -> Result<()>) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let url = format!("http://{}/artifact.tar.gz", listener.local_addr()?);

        std::thread::scope(|scope| {
            let server = scope.spawn(move || -> Result<()> {
                let (_stream, _address) = listener.accept()?;
                std::thread::sleep(Duration::from_millis(250));

                Ok(())
            });

            let operation_result = operation(url);
            let server_result = server
                .join()
                .map_err(|_panic| anyhow::anyhow!("test HTTP server thread panicked"))?;
            operation_result?;
            server_result?;

            Ok(())
        })
    }
}
