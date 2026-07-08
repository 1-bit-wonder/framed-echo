//! Integration test: a real echo server on an ephemeral port, a real TCP
//! client, and an assertion that the echoed bytes match what was sent.

use bytes::Bytes;
use framed_echo::FrameCodec;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::Framed;

#[tokio::test]
async fn echo_round_trip_over_tcp() {
    // Bind to port 0 so the OS picks a free ephemeral port for us.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Minimal echo server: accept one connection and echo frames until EOF.
    let server = tokio::spawn(async move {
        let (stream, _peer) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, FrameCodec::new());
        while let Some(frame) = framed.next().await {
            let payload = frame.unwrap();
            framed.send(payload).await.unwrap();
        }
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut client = Framed::new(stream, FrameCodec::new());

    let payloads: Vec<Bytes> = vec![
        Bytes::from_static(b""),
        Bytes::from_static(b"hello, framed world"),
        Bytes::from(vec![0xABu8; 100_000]), // spans many TCP reads
    ];

    for payload in &payloads {
        client.send(payload.clone()).await.unwrap();
        let echo = client.next().await.unwrap().unwrap();
        assert_eq!(&echo, payload, "echoed frame did not match sent frame");
    }

    // Dropping the client closes the socket; the server sees EOF and its task
    // completes cleanly.
    drop(client);
    server.await.unwrap();
}

#[tokio::test]
async fn many_small_frames_stay_ordered_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _peer) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, FrameCodec::new());
        while let Some(frame) = framed.next().await {
            framed.send(frame.unwrap()).await.unwrap();
        }
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut client = Framed::new(stream, FrameCodec::new());

    // Send a burst of tiny frames, then read them all back in order. This
    // exercises the "many frames glued into one read" path.
    const N: usize = 1000;
    for i in 0..N {
        client
            .send(Bytes::from(format!("frame-{i}")))
            .await
            .unwrap();
    }
    for i in 0..N {
        let echo = client.next().await.unwrap().unwrap();
        assert_eq!(&echo, &Bytes::from(format!("frame-{i}")));
    }

    drop(client);
    server.await.unwrap();
}
