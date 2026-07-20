use std::io;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::ipc::{
    LiveTurnControl, SimulatorHandler,
    proto::{IPC_VERSION, IpcError, SimulatorRequest, SimulatorResponse},
};

pub const MAX_IPC_REQUEST_BYTES: usize = 1_048_576;

pub async fn serve(bind: &str, session_token: impl Into<String>) -> io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    serve_listener(listener, session_token).await
}

/// Serve with an optional persistent recording store. When `database_path` is
/// set, the served handler persists each committed tick so an external simulator
/// process can recover its snapshot and event cursor after a real restart.
pub async fn serve_persistent(
    bind: &str,
    session_token: impl Into<String>,
    database_path: Option<&str>,
) -> io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    match database_path {
        Some(path) => {
            let handler =
                SimulatorHandler::new_persistent(session_token, path).map_err(io::Error::other)?;
            serve_listener_with(listener, handler).await
        }
        None => serve_listener(listener, session_token).await,
    }
}

pub async fn serve_listener(
    listener: TcpListener,
    session_token: impl Into<String>,
) -> io::Result<()> {
    serve_listener_with(listener, SimulatorHandler::new(session_token)).await
}

pub async fn serve_listener_with(
    listener: TcpListener,
    handler: SimulatorHandler,
) -> io::Result<()> {
    let session_token = handler.session_token().to_string();
    let live_turn_control = handler.live_turn_control();
    let handler = Arc::new(Mutex::new(handler));
    loop {
        let (stream, _) = listener.accept().await?;
        let handler = Arc::clone(&handler);
        let session_token = session_token.clone();
        let live_turn_control = live_turn_control.clone();
        tokio::spawn(async move {
            if let Err(error) =
                handle_connection(stream, handler, session_token, live_turn_control).await
            {
                eprintln!("cockpit-simulator connection closed: {error}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    handler: Arc<Mutex<SimulatorHandler>>,
    session_token: String,
    live_turn_control: LiveTurnControl,
) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = read;
    let mut buffer = Vec::with_capacity(8_192);
    while let Some(frame) = read_request_frame(&mut read, &mut buffer).await? {
        let oversized = matches!(frame, RequestFrame::Oversized);
        let response = match frame {
            RequestFrame::Oversized => payload_too_large_response(),
            RequestFrame::Data(bytes) => match serde_json::from_slice::<SimulatorRequest>(&bytes) {
                Ok(request)
                    if matches!(
                        &request.command,
                        crate::ipc::proto::SimulatorCommand::CancelLiveTurn
                    ) =>
                {
                    cancel_live_turn_response(request, &session_token, &live_turn_control)
                }
                Ok(request)
                    if matches!(
                        &request.command,
                        crate::ipc::proto::SimulatorCommand::Ping { .. }
                    ) =>
                {
                    ping_response(request, &session_token)
                }
                Ok(request) => handler.lock().await.dispatch_async(request).await,
                Err(error) => SimulatorResponse {
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

fn cancel_live_turn_response(
    request: SimulatorRequest,
    session_token: &str,
    live_turn_control: &LiveTurnControl,
) -> SimulatorResponse {
    if request.version != IPC_VERSION {
        return invalid_control_response(
            request.correlation_id,
            "IPC_VERSION_UNSUPPORTED",
            format!("supported IPC version is {IPC_VERSION}"),
        );
    }
    if request.session_token != session_token {
        return invalid_control_response(
            request.correlation_id,
            "SESSION_UNAUTHORIZED",
            "session token is invalid".to_string(),
        );
    }
    SimulatorResponse {
        version: IPC_VERSION,
        correlation_id: request.correlation_id,
        ok: true,
        result: Some(serde_json::json!({ "cancelled": live_turn_control.cancel() })),
        error: None,
    }
}

fn ping_response(request: SimulatorRequest, session_token: &str) -> SimulatorResponse {
    if request.version != IPC_VERSION {
        return invalid_control_response(
            request.correlation_id,
            "IPC_VERSION_UNSUPPORTED",
            format!("supported IPC version is {IPC_VERSION}"),
        );
    }
    if request.session_token != session_token {
        return invalid_control_response(
            request.correlation_id,
            "SESSION_UNAUTHORIZED",
            "session token is invalid".to_string(),
        );
    }
    let seq = match request.command {
        crate::ipc::proto::SimulatorCommand::Ping { seq } => seq,
        _ => unreachable!("ping_response only accepts Ping requests"),
    };
    SimulatorResponse {
        version: IPC_VERSION,
        correlation_id: request.correlation_id,
        ok: true,
        result: Some(serde_json::json!({ "pong": true, "seq": seq })),
        error: None,
    }
}

fn invalid_control_response(
    correlation_id: String,
    code: &str,
    message: String,
) -> SimulatorResponse {
    SimulatorResponse {
        version: IPC_VERSION,
        correlation_id: correlation_id.clone(),
        ok: false,
        result: None,
        error: Some(IpcError {
            code: code.to_string(),
            message,
            details: None,
            run_id: None,
            tick: None,
            correlation_id,
        }),
    }
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

fn payload_too_large_response() -> SimulatorResponse {
    SimulatorResponse {
        version: IPC_VERSION,
        correlation_id: "payload-too-large".to_string(),
        ok: false,
        result: None,
        error: Some(IpcError {
            code: "PAYLOAD_TOO_LARGE".to_string(),
            message: format!("simulator request exceeds {MAX_IPC_REQUEST_BYTES} byte limit"),
            details: Some(serde_json::json!({ "maxBytes": MAX_IPC_REQUEST_BYTES })),
            run_id: None,
            tick: None,
            correlation_id: "payload-too-large".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::proto::{SimulatorCommand, SimulatorRequest};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        time::{Duration, timeout},
    };

    #[tokio::test]
    async fn cancel_live_turn_bypasses_a_busy_simulator_handler() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("listener address");
        let control = LiveTurnControl::default();
        let token = control.begin();
        let handler = Arc::new(Mutex::new(SimulatorHandler::with_live_turn_control(
            "test-token",
            control.clone(),
        )));
        let handler_lock = handler.lock().await;
        let server = tokio::spawn({
            let handler = Arc::clone(&handler);
            let control = control.clone();
            async move {
                let (stream, _) = listener.accept().await.expect("connection accepted");
                handle_connection(stream, handler, "test-token".to_string(), control)
                    .await
                    .expect("connection completes");
            }
        });

        let mut stream = TcpStream::connect(address).await.expect("client connects");
        let request = SimulatorRequest {
            version: IPC_VERSION,
            session_token: "test-token".to_string(),
            correlation_id: "cancel-test".to_string(),
            command: SimulatorCommand::CancelLiveTurn,
        };
        let mut encoded = serde_json::to_vec(&request).expect("request serializes");
        encoded.push(b'\n');
        stream.write_all(&encoded).await.expect("request writes");

        let mut line = String::new();
        timeout(
            Duration::from_millis(250),
            BufReader::new(stream).read_line(&mut line),
        )
        .await
        .expect("cancel response is not blocked by the handler lock")
        .expect("cancel response reads");
        let response: SimulatorResponse = serde_json::from_str(&line).expect("response parses");
        assert!(response.ok);
        assert!(token.is_cancelled());

        drop(handler_lock);
        server.await.expect("server task joins");
    }

    #[tokio::test]
    async fn ping_bypasses_a_busy_simulator_handler() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("listener address");
        let control = LiveTurnControl::default();
        let handler = Arc::new(Mutex::new(SimulatorHandler::with_live_turn_control(
            "test-token",
            control.clone(),
        )));
        let handler_lock = handler.lock().await;
        let server = tokio::spawn({
            let handler = Arc::clone(&handler);
            async move {
                let (stream, _) = listener.accept().await.expect("connection accepted");
                handle_connection(stream, handler, "test-token".to_string(), control)
                    .await
                    .expect("connection completes");
            }
        });

        let mut stream = TcpStream::connect(address).await.expect("client connects");
        let request = SimulatorRequest {
            version: IPC_VERSION,
            session_token: "test-token".to_string(),
            correlation_id: "ping-test".to_string(),
            command: SimulatorCommand::Ping { seq: 9 },
        };
        let mut encoded = serde_json::to_vec(&request).expect("request serializes");
        encoded.push(b'\n');
        stream.write_all(&encoded).await.expect("request writes");

        let mut line = String::new();
        timeout(
            Duration::from_millis(250),
            BufReader::new(&mut stream).read_line(&mut line),
        )
        .await
        .expect("ping response is not blocked by the handler lock")
        .expect("ping response reads");
        let response: SimulatorResponse = serde_json::from_str(&line).expect("response parses");
        assert!(response.ok);
        assert_eq!(
            response
                .result
                .as_ref()
                .and_then(|result| result.get("seq"))
                .and_then(serde_json::Value::as_u64),
            Some(9)
        );

        drop(stream);
        drop(handler_lock);
        server.await.expect("server task joins");
    }
}
