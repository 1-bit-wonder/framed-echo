//! Async echo server: one Tokio task per connection, each stream wrapped in a
//! `Framed` codec. A decoded frame is echoed straight back, re-encoded.

use std::env;

use bytes::Bytes;
use framed_echo::{FrameCodec, FrameError};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::Framed;

const DEFAULT_ADDR: &str = "127.0.0.1:9000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Bind address: first CLI arg, else `FRAMED_ECHO_ADDR`, else the default.
    let addr = env::args()
        .nth(1)
        .or_else(|| env::var("FRAMED_ECHO_ADDR").ok())
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());

    let listener = TcpListener::bind(&addr).await?;
    println!("framed-echo server listening on {}", listener.local_addr()?);

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                // One task per connection. A single client's error or EOF never
                // touches the accept loop or any other connection.
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream).await {
                        eprintln!("connection {peer} closed with error: {e}");
                    }
                });
            }
            // A transient accept error (e.g. fd exhaustion) is logged, not
            // fatal — the server keeps serving existing and future clients.
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

async fn handle_connection(stream: TcpStream) -> Result<(), FrameError> {
    let mut framed = Framed::new(stream, FrameCodec::new());

    // Each `next()` yields exactly one fully-decoded frame. A clean EOF ends
    // the stream with `None`, so the loop exits without an error.
    while let Some(frame) = framed.next().await {
        let payload: Bytes = frame?;
        framed.send(payload).await?;
    }

    Ok(())
}
