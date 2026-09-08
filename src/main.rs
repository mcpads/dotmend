mod invocations;
mod server;
mod server_instructions;
mod workbench;

use dotmend::workspace::Workspace;
use rmcp::ServiceExt;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut root = PathBuf::from(".");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => root = args.next().ok_or("--workspace requires a path")?.into(),
            "--help" | "-h" => {
                println!(
                    "dotmend [--workspace PATH]\nMCP stdio server. Open the human screen with open_workbench; no standalone web process."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    let workspace = Arc::new(Mutex::new(Workspace::open(&root)?));
    let root = root.canonicalize()?;
    let workbench = Arc::new(tokio::sync::Mutex::new(workbench::Workbench::new(root)));
    let service = server::ArtServer {
        workspace,
        workbench,
        invocations: Arc::new(invocations::Invocations::new()),
    }
    .serve(rmcp::transport::stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}
