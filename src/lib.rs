//! `framed-echo`: a zero-copy, length-prefixed framing codec over Tokio.
//!
//! TCP is a byte stream with no message boundaries, so any application that
//! sends discrete messages must add its own framing. This crate implements the
//! standard solution — a length prefix followed by a payload — as a
//! [`tokio_util::codec`] [`Decoder`]/[`Encoder`] pair.
//!
//! Two mechanics are worth attention, both documented inline in [`codec`]:
//!
//! - **Partial frames.** A single socket read may contain half a frame or
//!   several; the decoder detects an incomplete frame, returns `Ok(None)`, and
//!   keeps the buffered bytes until the rest arrives.
//! - **Zero-copy payloads.** Decoded payloads are returned as [`bytes::Bytes`],
//!   a refcounted view into the receive buffer, so there is no per-message
//!   allocation or copy.
//!
//! [`Decoder`]: tokio_util::codec::Decoder
//! [`Encoder`]: tokio_util::codec::Encoder

mod codec;

pub use codec::{FrameCodec, FrameError, DEFAULT_MAX_FRAME_LEN};
