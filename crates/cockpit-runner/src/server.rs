use std::io;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::ipc::{
    RunnerHandler,
    proto::{IPC_VERSION, IpcError, RunnerRequest, RunnerResponse},
};

pub const MAX_IPC_REQUEST_BYTES: usize = 1_048_576;

pub async fn serve(bind: &str, session_token: impl Into<String>) -> io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    serve_listener(listener, session_token).await
}

pub async fn serve_listener(
    listener: TcpListener,
    session_token: impl Into<String>,
) -> io::Result<()> {
    let handler = Arc::new(Mutex::new(RunnerHandler::new(session_token)));
    loop {
        let (stream, _) = listener.accept().await?;
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, handler).await {
                eprintln!("cockpit-runner connection closed: {error}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    handler: Arc<Mutex<RunnerHandler>>,
) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = read;
    let mut buffer = Vec::with_capacity(8_192);
    while let Some(frame) = read_request_frame(&mut read, &mut buffer).await? {
        let oversized = matches!(frame, RequestFrame::Oversized);
        let response = match frame {
            RequestFrame::Oversized => payload_too_large_response(),
            RequestFrame::Data(bytes) => match serde_json::from_slice::<RunnerRequest>(&bytes) {
                Ok(request) => handler.lock().await.dispatch(request),
                Err(error) => RunnerResponse {
                    version: IPC_VERSION,
                    correlation_id: "invalid-request".to_string(),
                    ok: false,
                    result: None,
                    error: Some(IpcError {
                        code: "INVALID_REQUEST".to_string(),
                        message: error.to_string(),
                        details: None,
                        run_id: None,
                        tick: None,
                        correlation_id: "invalid-request".to_string(),
                    }),
                },
            },
        };
        let mut encoded =
            serde_json::to_vec(&response).map_err(|error| io::Error::other(error.to_string()))?;
        encoded.push(b'\n');
        write.write_all(&encoded).await?;
        write.flush().await?;
        if oversized {
            break;
        }
    }
    Ok(())
}

enum RequestFrame {
    Data(Vec<u8>),
    Oversized,
}

async fn read_request_frame(
    read: &mut tokio::net::tcp::OwnedReadHalf,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<RequestFrame>> {
    let mut chunk = [0_u8; 8_192];
    loop {
        if let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            if newline > MAX_IPC_REQUEST_BYTES {
                return Ok(Some(RequestFrame::Oversized));
            }
            let frame = buffer.drain(..=newline).collect::<Vec<_>>();
            return Ok(Some(RequestFrame::Data(frame[..newline].to_vec())));
        }
        if buffer.len() > MAX_IPC_REQUEST_BYTES {
            return Ok(Some(RequestFrame::Oversized));
        }
        let read_count = read.read(&mut chunk).await?;
        if read_count == 0 {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Ok(Some(RequestFrame::Data(std::mem::take(buffer))))
            };
        }
        buffer.extend_from_slice(&chunk[..read_count]);
    }
}

fn payload_too_large_response() -> RunnerResponse {
    RunnerResponse {
        version: IPC_VERSION,
        correlation_id: "payload-too-large".to_string(),
        ok: false,
        result: None,
        error: Some(IpcError {
            code: "PAYLOAD_TOO_LARGE".to_string(),
            message: format!("runner request exceeds {MAX_IPC_REQUEST_BYTES} byte limit"),
            details: Some(serde_json::json!({ "maxBytes": MAX_IPC_REQUEST_BYTES })),
            run_id: None,
            tick: None,
            correlation_id: "payload-too-large".to_string(),
        }),
    }
}
