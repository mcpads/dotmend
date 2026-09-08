# Install Dotmend for the user

This file is for the agent performing installation. Handle setup and verification yourself. Ask the user only for information or access that you cannot obtain, such as their intended asset workspace. Do not turn these steps into a checklist for the human.

## Choose the release

1. Inspect the host OS, CPU architecture, existing Dotmend installation, and the user's MCP client. Preserve existing configuration and workspaces. Use a writable, persistent asset workspace; never use the installation directory as the workspace.
2. Read the latest published release from `https://api.github.com/repos/mcpads/dotmend/releases/latest` (or use `gh release view --repo mcpads/dotmend`). Pin its tag and asset URLs for the entire installation. Do not install a draft or build from `next` unless the user requests development source.
3. Download the matching archive and `SHA256SUMS` from that same release. Compute SHA-256 locally and require an exact match for the archive filename before extracting or executing it. A checksum detects corrupted or mismatched downloads; it is not a code signature.

| Host | Asset |
| --- | --- |
| Windows x64 | `dotmend-x86_64-pc-windows-msvc.zip` |
| macOS 26 or newer, Apple Silicon | `dotmend-aarch64-apple-darwin.tar.gz` |
| Linux x64 with glibc 2.35 or newer | `dotmend-x86_64-unknown-linux-gnu.tar.gz` |

macOS builds target and are tested on macOS 26; Intel Macs are outside the supported targets. Windows builds are tested on Windows Server 2022; Linux builds on Ubuntu 22.04. The binaries are not signed or notarized. Do not disable OS security controls to launch them; if launch is blocked, use an authorized source build or explain the specific OS approval needed. Other architectures and musl Linux require a source build and local verification.

## Install and connect

Extract into a temporary directory, then place `dotmend` (Windows: `dotmend.exe`) in a persistent, user-writable application directory. Preserve executable permissions on Unix. Resolve its absolute path rather than relying on the MCP client's `PATH`. Keep the prior executable available until the replacement connects successfully; do not overwrite a running Windows executable.

Run the executable with `--version` and check that it matches the selected release tag without its leading `v`. Register one MCP stdio server named `dotmend`, using the client's supported configuration mechanism:

- **Command:** the absolute path of the installed executable.
- **Arguments:** `--workspace` followed by the absolute path of the asset workspace, as separate arguments.
- **Transport:** stdio; keep stdout exclusively for JSON-RPC.
- **Protocol:** stateless MCP `2026-07-28`. The client must support this protocol and supply its required per-request metadata. A client that only sends `initialize` cannot connect. Check the installed client's help and current official documentation for any required feature settings; do not guess configuration keys or silently downgrade the protocol.

Merge the entry into existing configuration without removing other servers. Reuse an existing entry for this installation instead of accumulating duplicate registrations. Reconnect the MCP client after changing the executable or configuration. If the client cannot support the required protocol, report that specific limitation and leave its other servers intact.

## Verify before reporting success

Use the installed executable and a temporary workspace first. Confirm `server/discover`, `tools/list`, and `resources/read` for `dotmend://guides/editing`. On each JSON-RPC request include this object in `params._meta`:

```json
{
  "io.modelcontextprotocol/protocolVersion": "2026-07-28",
  "io.modelcontextprotocol/clientCapabilities": {}
}
```

The repository's `scripts/check_mcp.py` performs these checks, creates a tiny synthetic asset, and opens and closes its managed browser server using the supplied executable. Run it with Python 3.11 or newer if available, or perform the equivalent checks through MCP. It does not require a source build or touch the user's assets.

Finally verify tool discovery in the user's actual client and read the editing guide there. Use `open_workbench` to prepare the user's screen and give them the returned URL when they want to edit. Manage its lifetime with `inspect_workbench` and `close_workbench`; never launch standalone web processes or isolate runtime lock directories to bypass limits.

Report the installed release, client connection result, and the next action in plain language. An executable starting successfully is not proof that the client connected.

## Source builds when needed

For a source build, check out the selected release tag. Install stable Rust and the host C toolchain for bundled SQLite (MSVC on Windows, Xcode Command Line Tools on macOS, the distribution's C build tools on Linux). Run `cargo build --release --locked` and verify `target/release/dotmend` (`dotmend.exe` on Windows) as above. The browser UI and editing guide are embedded in the executable. Node.js is needed only for browser tests.

Development happens on `next`; contributors build and test locally. CI runs for `main` pushes and pull requests targeting `main`, and the release workflow validates tagged commits from `main`. Versions identify published releases; `next` does not receive an automatic version bump.
