//! Demo client: connects, sends frames of varying sizes, and verifies each
//! echoed frame matches what was sent.

use std::env;

use bytes::Bytes;
use framed_echo::FrameCodec;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

const DEFAULT_ADDR: &str = "127.0.0.1:9000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = env::args()
        .nth(1)
        .or_else(|| env::var("FRAMED_ECHO_ADDR").ok())
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());

    let stream = TcpStream::connect(&addr).await?;
    let mut framed = Framed::new(stream, FrameCodec::new());

    // A spread of payload sizes: empty, tiny, and up to 1 MiB. These exercise
    // both the single-read and split-across-many-reads paths on the server.
    let sizes = [0usize, 1, 16, 1024, 64 * 1024, 1024 * 1024];

    for (i, &size) in sizes.iter().enumerate() {
        // A deterministic, size- and index-dependent pattern so any mismatch
        // (truncation, misframing, off-by-one) is easy to spot.
        let payload: Bytes = (0..size).map(|b| (b as u8).wrapping_add(i as u8)).collect();

        framed.send(payload.clone()).await?;

        match framed.next().await {
            Some(Ok(echo)) if echo == payload => {
                println!("frame {i}: {size} bytes echoed correctly");
            }
            Some(Ok(echo)) => {
                return Err(format!(
                    "frame {i}: echo mismatch (got {} bytes, expected {})",
                    echo.len(),
                    payload.len()
                )
                .into());
            }
            Some(Err(e)) => return Err(e.into()),
            None => return Err("server closed the connection early".into()),
        }
    }

    println!("all {} frames verified", sizes.len());
    Ok(())
}
