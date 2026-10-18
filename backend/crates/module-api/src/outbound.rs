//! The one way the core and the modules make outgoing requests: bounded in time and in size, and
//! never logging a request or response body.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum OutboundError {
    #[error("cannot build the HTTP client: {0}")]
    Build(String),
    #[error("request failed: {0}")]
    Request(String),
    #[error("the response is larger than {limit} bytes")]
    TooLarge { limit: usize },
}

pub struct OutboundClient {
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl OutboundClient {
    /// `redirects`: how many redirects to follow (0: none, so a key is never sent elsewhere).
    pub fn new(
        connect_timeout: Duration,
        total_timeout: Duration,
        max_response_bytes: usize,
        redirects: usize,
    ) -> Result<Self, OutboundError> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(total_timeout)
            .redirect(if redirects == 0 {
                reqwest::redirect::Policy::none()
            } else {
                reqwest::redirect::Policy::limited(redirects)
            })
            .build()
            .map_err(|e| OutboundError::Build(e.to_string()))?;
        Ok(Self {
            client,
            max_response_bytes,
        })
    }

    /// GET `url`; returns the status and the body, at most `max_response_bytes` of it.
    pub async fn get(&self, url: &str) -> Result<(u16, Vec<u8>), OutboundError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| OutboundError::Request(without_url(e)))?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|len| len > self.max_response_bytes as u64)
        {
            return Err(OutboundError::TooLarge {
                limit: self.max_response_bytes,
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| OutboundError::Request(without_url(e)))?
        {
            if body.len() + chunk.len() > self.max_response_bytes {
                return Err(OutboundError::TooLarge {
                    limit: self.max_response_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }

    /// Whether something answers at `url`: the status of a GET, with the body thrown away. For
    /// `doctor`; it sends no user data.
    pub async fn probe(&self, url: &str) -> Result<u16, OutboundError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| OutboundError::Request(without_url(e)))?;
        Ok(response.status().as_u16())
    }
}

/// A reqwest error mentions the URL, which can carry a key in its query. Say what went wrong
/// without it.
fn without_url(error: reqwest::Error) -> String {
    error.without_url().to_string()
}
