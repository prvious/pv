use std::io::Write;

use crate::{ResourcesError, Result};

pub trait ResourceHttpClient {
    fn get_text(&self, url: &str) -> Result<String>;

    fn download(&self, url: &str, writer: &mut dyn Write) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UreqResourceHttpClient;

impl ResourceHttpClient for UreqResourceHttpClient {
    fn get_text(&self, url: &str) -> Result<String> {
        let mut response = ureq::get(url)
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
        let mut response = ureq::get(url)
            .call()
            .map_err(|source| http_error(url, source))?;

        std::io::copy(&mut response.body_mut().as_reader(), writer)
            .map(|_bytes| ())
            .map_err(|source| ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: source.to_string(),
            })
    }
}

fn http_error(url: &str, source: ureq::Error) -> ResourcesError {
    ResourcesError::HttpRequestFailed {
        url: url.to_string(),
        reason: source.to_string(),
    }
}
