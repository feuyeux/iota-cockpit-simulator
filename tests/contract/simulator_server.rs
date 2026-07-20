use cockpit_simulator::{
    ipc::proto::{IPC_VERSION, SimulatorCommand, SimulatorRequest},
    server::{MAX_IPC_REQUEST_BYTES, serve_listener},
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

async fn call(
    write: &mut tokio::net::tcp::OwnedWriteHalf,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    command: SimulatorCommand,
) -> Value {
    let request = SimulatorRequest {
        version: IPC_VERSION,
        session_token: "server-test-token".to_string(),
        correlation_id: "server-test-correlation".to_string(),
        command,
    };
    let mut encoded = serde_json::to_vec(&request).expect("request serializes");
    encoded.push(b'\n');
    write.write_all(&encoded).await.expect("request writes");
    let line = lines
        .next_line()
        .await
        .expect("response reads")
        .expect("response exists");
    serde_json::from_str(&line).expect("response parses")
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_server_rejects_oversized_request_frames() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address exists");
    let server = tokio::spawn(serve_listener(listener, "server-test-token"));
    let stream = TcpStream::connect(address).await.expect("connection");
    let (read, mut write) = stream.into_split();
    let mut payload = vec![b'x'; MAX_IPC_REQUEST_BYTES + 1];
    payload.push(b'\n');
    write
        .write_all(&payload)
        .await
        .expect("oversized request writes");
    let mut lines = BufReader::new(read).lines();
    let line = lines
        .next_line()
        .await
        .expect("response reads")
        .expect("response exists");
    let response: Value = serde_json::from_str(&line).expect("response parses");
    assert_eq!(
        response
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("PAYLOAD_TOO_LARGE")
    );
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_server_preserves_state_across_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address exists");
    let server = tokio::spawn(serve_listener(listener, "server-test-token"));

    let stream = TcpStream::connect(address).await.expect("first connection");
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let created = call(
        &mut write,
        &mut lines,
        SimulatorCommand::CreateSimulationRun {
            path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        },
    )
    .await;
    assert_eq!(created.get("ok").and_then(Value::as_bool), Some(true));
    let stepped = call(&mut write, &mut lines, SimulatorCommand::StepSimulation).await;
    assert_eq!(stepped.get("ok").and_then(Value::as_bool), Some(true));
    drop(write);
    drop(lines);

    let stream = TcpStream::connect(address).await.expect("reconnect");
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let events = call(
        &mut write,
        &mut lines,
        SimulatorCommand::GetSimulationEvents { cursor: Some(0) },
    )
    .await;
    assert_eq!(events.get("ok").and_then(Value::as_bool), Some(true));
    let event_list = events
        .get("result")
        .and_then(|result| result.get("events"))
        .and_then(Value::as_array)
        .expect("event list");
    assert!(event_list.iter().any(|event| {
        event.get("type") == Some(&Value::String("SimulationTickCommitted".to_string()))
    }));
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_server_answers_authenticated_ping_without_simulation_state() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address exists");
    let server = tokio::spawn(serve_listener(listener, "server-test-token"));

    let stream = TcpStream::connect(address).await.expect("connection");
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let response = call(&mut write, &mut lines, SimulatorCommand::Ping { seq: 42 }).await;

    assert_eq!(
        response.get("version").and_then(Value::as_u64),
        Some(IPC_VERSION as u64)
    );
    assert_eq!(response.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(
        response
            .get("result")
            .and_then(|result| result.get("pong"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        response
            .get("result")
            .and_then(|result| result.get("seq"))
            .and_then(Value::as_u64),
        Some(42)
    );
    server.abort();
}
