use std::{
    pin::Pin,
    task::{Context, Poll},
};

use axum::body::Bytes;
use axum::{response::sse::Event, Error};
use futures::Stream;
use reqwest;
use serde_json::Value;
use tracing::error;

/// A structure for streaming chat completion chunks.
pub struct Streamer {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    /// Current status of the stream
    status: StreamStatus,
}

/// Represents the various states of a streaming process
#[derive(Debug, PartialEq, Eq)]
pub enum StreamStatus {
    /// Stream has not started
    NotStarted,
    /// Stream is actively receiving data
    Started,
    /// Stream has completed successfully
    Completed,
    /// Stream failed with an error
    Failed(String),
}

impl Streamer {
    /// Creates a new Streamer instance
    pub fn new(stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self {
            stream: Box::pin(stream),
            status: StreamStatus::NotStarted,
        }
    }
}

impl Stream for Streamer {
    type Item = Result<Event, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.status == StreamStatus::Completed {
            return Poll::Ready(None);
        }

        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                if self.status != StreamStatus::Started {
                    self.status = StreamStatus::Started;
                }

                let chunk = serde_json::from_slice::<Value>(&chunk).map_err(|e| {
                    error!("Error parsing chunk: {}", e);
                    Error::new(format!("Error parsing chunk: {}", e))
                })?;
                Poll::Ready(Some(Ok(Event::default().json_data(&chunk)?)))
            }
            Poll::Ready(Some(Err(e))) => {
                self.status = StreamStatus::Failed(e.to_string());
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                self.status = StreamStatus::Completed;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
