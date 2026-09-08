# Dotmend

**Pixel art for retro game assets, made with an agent and refined by you.**

Dotmend gives AI agents tools to create and edit pixel art within a supplied palette, image size, and resource constraints. It is the art layer of [RetroIR](https://mcpads.dev/).

Tell your agent what you want to make or change. It can inspect the artwork, focus on a small area, protect details, compare related assets, and prepare a browser view with originals or references alongside the result.

When a detail needs your touch, the screen has four actions:

- Pick a palette color and click or drag over pixels.
- Click **Mark issues** to flag pixels: left-click or drag to add marks, right-click or drag to remove them. Choose a palette color to return to painting.
- Undo the last stroke with **Ctrl/⌘+Z**.
- Save with **Ctrl/⌘+S**.

Marks are saved separately from the artwork so your agent can inspect the exact locations.

Ask the agent to show a collection, filter the view, bring back an earlier candidate, or work on a particular area. You do not need to learn an editor to do that.

## What it supports

- Start from exact palette indices, a blank canvas, or an imported PNG.
- Make local edits while preserving protected pixels and palette index meaning.
- Compare candidates at their actual size and with crisp pixel enlargement.
- Preview and apply the same edit to explicitly selected, compatible assets.
- Keep intermediate results and resume or branch from earlier candidates.
- Validate the supplied constraints and export artwork with its verification and provenance.

Images from external generators, including Imagen, can enter the same workflow through PNG import. Generation and image conversion are explicit steps; the final artwork still needs to meet the target's constraints.

Dotmend runs as a native Rust MCP server with a local browser interface. The agent manages that interface through MCP tools. Current client connections use the stateless MCP protocol `2026-07-28`.

It focuses on making and modifying retro game assets. Font rendering and layer composition are planned; ROM insertion and verification inside a running game belong to the surrounding toolchain.

## Get started with your agent

Give your agent this repository and ask:

> Install Dotmend from https://github.com/mcpads/dotmend. Follow `INSTALL.md`, connect it to my MCP client, and verify that it works. Then help me open an asset to edit.

Your agent handles the download, setup, and connection checks. Releases provide native builds for Windows, macOS, and Linux. You do not need to build the app or run terminal commands yourself.
