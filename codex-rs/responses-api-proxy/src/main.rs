use clap::Parser;
use codex_responses_api_proxy::Args as ResponsesApiProxyArgs;

#[ctor::ctor]
fn pre_main() {
    codex_process_hardening::pre_main_hardening();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let args = ResponsesApiProxyArgs::parse();
    codex_responses_api_proxy::run_main(args).await
}
