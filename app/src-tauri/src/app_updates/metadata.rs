//! The application's response/deadline policy, independent of sockets and TLS.
use super::policy;
use std::future::Future;

pub(super) trait Response {
    fn status(&self) -> u16;
    fn content_length(&self) -> Option<u64>;
    fn next(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, String>> + Send;
}

pub(super) async fn read<R: Response>(
    request: impl Future<Output = Result<R, String>>,
) -> Result<Vec<u8>, String> {
    tokio::time::timeout(policy::DEADLINE, async {
        let mut response = request.await?;
        if response.status() != 200 {
            return Err(format!("release HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > policy::MAX_BODY as u64)
        {
            return Err("release response too large".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.next().await? {
            if body.len().saturating_add(chunk.len()) > policy::MAX_BODY {
                return Err("release response too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    })
    .await
    .map_err(|_| "release request timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        future::{pending, ready},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    struct MockResponse {
        status: u16,
        length: Option<u64>,
        chunks: VecDeque<Result<Vec<u8>, String>>,
        stalled: bool,
        reads: Arc<AtomicUsize>,
    }
    impl MockResponse {
        fn new(status: u16, chunks: Vec<Result<Vec<u8>, String>>) -> Self {
            Self {
                status,
                length: None,
                chunks: chunks.into(),
                stalled: false,
                reads: Arc::new(AtomicUsize::new(0)),
            }
        }
    }
    impl Response for MockResponse {
        fn status(&self) -> u16 {
            self.status
        }
        fn content_length(&self) -> Option<u64> {
            self.length
        }
        async fn next(&mut self) -> Result<Option<Vec<u8>>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.stalled {
                pending::<()>().await;
            }
            self.chunks.pop_front().transpose()
        }
    }
    fn run(test: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(test);
    }
    #[test]
    fn successful_chunks_and_exact_limit_preserve_bytes() {
        run(async {
            let response = MockResponse::new(200, vec![Ok(b"{".to_vec()), Ok(b"}".to_vec())]);
            assert_eq!(read(ready(Ok(response))).await.unwrap(), b"{}");
            let mut response = MockResponse::new(200, vec![Ok(vec![b'x'; policy::MAX_BODY])]);
            response.length = Some(policy::MAX_BODY as u64);
            assert_eq!(
                read(ready(Ok(response))).await.unwrap().len(),
                policy::MAX_BODY
            );
        });
    }
    #[test]
    fn errors_and_redirects_are_rejected_before_reading_the_body() {
        run(async {
            for status in [201, 204, 301, 302, 307, 308, 404, 429, 503] {
                let response = MockResponse::new(status, vec![Err("body must not be read".into())]);
                let reads = response.reads.clone();
                assert_eq!(
                    read(ready(Ok(response))).await,
                    Err(format!("release HTTP {status}"))
                );
                assert_eq!(
                    reads.load(Ordering::SeqCst),
                    0,
                    "HTTP {status} body was consumed"
                );
            }
        });
    }
    #[test]
    fn declared_and_streamed_sizes_are_bounded_even_without_content_length() {
        run(async {
            let mut response = MockResponse::new(200, vec![]);
            response.length = Some(policy::MAX_BODY as u64 + 1);
            let reads = response.reads.clone();
            assert_eq!(
                read(ready(Ok(response))).await,
                Err("release response too large".into())
            );
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            for length in [None, Some(1)] {
                let mut response =
                    MockResponse::new(200, vec![Ok(vec![b'x'; policy::MAX_BODY]), Ok(vec![b'x'])]);
                response.length = length;
                assert_eq!(
                    read(ready(Ok(response))).await,
                    Err("release response too large".into())
                );
            }
        });
    }
    #[test]
    fn request_and_mid_body_failures_retain_the_actual_diagnostic() {
        run(async {
            assert_eq!(
                read::<MockResponse>(ready(Err("synthetic connect failure".into()))).await,
                Err("synthetic connect failure".into())
            );
            let response = MockResponse::new(
                200,
                vec![
                    Ok(b"partial".to_vec()),
                    Err("synthetic truncated body".into()),
                ],
            );
            assert_eq!(
                read(ready(Ok(response))).await,
                Err("synthetic truncated body".into())
            );
        });
    }
    #[test]
    fn one_virtual_deadline_covers_both_request_and_body() {
        run(async {
            for stalled_request in [true, false] {
                let response = MockResponse {
                    stalled: true,
                    ..MockResponse::new(200, vec![])
                };
                let started = tokio::time::Instant::now();
                let request = async move {
                    if stalled_request {
                        pending::<()>().await;
                    }
                    Ok(response)
                };
                assert_eq!(read(request).await, Err("release request timed out".into()));
                assert_eq!(started.elapsed(), policy::DEADLINE);
            }
        });
    }
}
