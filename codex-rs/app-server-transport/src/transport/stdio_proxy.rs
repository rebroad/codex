//! Bridge app-server stdio JSON-RPC messages over the daemon control WebSocket.

use std::io;
use std::path::Path;
use std::time::Duration;

use codex_uds::UnixStream;
use futures::SinkExt;
use futures::StreamExt;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::client_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONTROL_MESSAGE_SIZE: usize = 128 << 20;
const UDS_WEBSOCKET_HANDSHAKE_URL: &str = "ws://localhost/rpc";

/// Relay app-server JSON-RPC stdio messages through its local daemon WebSocket.
pub async fn run_stdio_to_control_socket(socket_path: &Path) -> io::Result<()> {
    let websocket = connect_control_socket(socket_path).await?;
    bridge_stdio(tokio::io::stdin(), tokio::io::stdout(), websocket).await
}

async fn connect_control_socket(socket_path: &Path) -> io::Result<WebSocketStream<UnixStream>> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket_path))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out connecting to app-server control socket",
            )
        })??;
    let request = UDS_WEBSOCKET_HANDSHAKE_URL
        .into_client_request()
        .map_err(io::Error::other)?;
    let config = WebSocketConfig::default()
        .max_frame_size(Some(MAX_CONTROL_MESSAGE_SIZE))
        .max_message_size(Some(MAX_CONTROL_MESSAGE_SIZE));
    tokio::time::timeout(
        CONNECT_TIMEOUT,
        client_async_with_config(request, stream, Some(config)),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "timed out upgrading app-server control socket",
        )
    })?
    .map(|(websocket, _)| websocket)
    .map_err(io::Error::other)
}

async fn bridge_stdio<R, W, S>(
    input: R,
    mut output: W,
    websocket: WebSocketStream<S>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(input).lines();
    let (mut websocket_writer, mut websocket_reader) = websocket.split();
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => websocket_writer
                    .send(Message::Text(line.into()))
                    .await
                    .map_err(io::Error::other)?,
                None => {
                    websocket_writer
                        .send(Message::Close(None))
                        .await
                        .map_err(io::Error::other)?;
                    return Ok(());
                }
            },
            message = websocket_reader.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    output.write_all(text.as_bytes()).await?;
                    output.write_all(b"\n").await?;
                    output.flush().await?;
                }
                Some(Ok(Message::Ping(payload))) => websocket_writer
                    .send(Message::Pong(payload))
                    .await
                    .map_err(io::Error::other)?,
                Some(Ok(Message::Pong(_) | Message::Frame(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    websocket_writer
                        .send(Message::Close(frame))
                        .await
                        .map_err(io::Error::other)?;
                    return Ok(());
                }
                Some(Ok(Message::Binary(_))) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "app-server control socket sent a binary message",
                    ));
                }
                Some(Err(error)) => return Err(io::Error::other(error)),
                None => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
#[path = "stdio_proxy_tests.rs"]
mod tests;
