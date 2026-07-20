use iota_core::AcpBackend;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    cockpit_judge::run_for_backend(AcpBackend::OpenCode).await
}
