use super::bridge_stdio;
use super::connect_control_socket;
use codex_uds::UnixListener;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use std::error::Error;
use std::io;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn stdio_proxy_upgrades_and_relays_json_rpc_messages()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = tempfile::TempDir::new()?;
    let socket_path = dir.path().join("app-server.sock");
    let mut listener = UnixListener::bind(&socket_path).await?;
    let server = tokio::spawn(async move {
        let stream = listener.accept().await?;
        let mut websocket = accept_async(stream).await?;
        let request = websocket
            .next()
            .await
            .ok_or_else(|| io::Error::other("stdio proxy closed before sending a request"))??;
        assert_eq!(
            request,
            Message::Text(r#"{"id":1,"method":"initialize"}"#.into())
        );
        websocket
            .send(Message::Text(r#"{"id":1,"result":{}}"#.into()))
            .await?;
        let close = websocket
            .next()
            .await
            .ok_or_else(|| io::Error::other("stdio proxy did not close the WebSocket"))??;
        assert!(matches!(close, Message::Close(_)));
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    let websocket = connect_control_socket(&socket_path).await?;
    let (input_writer, input_reader) = tokio::io::duplex(1024);
    let (output_writer, output_reader) = tokio::io::duplex(1024);
    let proxy = tokio::spawn(bridge_stdio(input_reader, output_writer, websocket));
    let mut input_writer = input_writer;
    input_writer
        .write_all(b"{\"id\":1,\"method\":\"initialize\"}\n")
        .await?;
    let mut output_lines = BufReader::new(output_reader).lines();
    assert_eq!(
        output_lines.next_line().await?,
        Some(r#"{"id":1,"result":{}}"#.to_string())
    );
    input_writer.shutdown().await?;
    proxy.await??;
    server.await??;
    Ok(())
}
