mod invocations;
mod server;
mod server_instructions;
mod workbench;

use rmcp::ServiceExt;
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut root = PathBuf::from(".");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => root = args.next().ok_or("--workspace requires a path")?.into(),
            "--version" | "-V" => {
                println!("dotmend {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "dotmend [--workspace PATH] [--version]\nMCP stdio server. Uses the launch directory as the project and stores work in .dotmend/. --workspace selects an explicit project. Open the human screen with open_workbench."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    let root = std::path::absolute(root)?;
    let workspace = Arc::new(server::DeferredWorkspace::new(root.clone()));
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
