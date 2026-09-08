"""Check an installed executable using a disposable workspace and real MCP IO."""

import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
from urllib.request import urlopen


def check(binary):
    binary = str(Path(binary).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="dotmend-check-") as workspace:
        with tempfile.TemporaryFile(mode="w+t") as stderr:
            child = subprocess.Popen(
                [binary, "--workspace", workspace],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
                text=True, encoding="utf-8",
                env={**os.environ, "DOTMEND_RUNTIME_DIR": str(Path(workspace) / "runtime")},
            )
            replies = queue.Queue()
            def read_replies():
                for line in child.stdout:
                    replies.put(line)

            reader = threading.Thread(target=read_replies, daemon=True)
            reader.start()
            sequence = 0

            def request(method, **params):
                nonlocal sequence
                sequence += 1
                params["_meta"] = {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {},
                }
                child.stdin.write(json.dumps({
                    "jsonrpc": "2.0", "id": sequence, "method": method, "params": params,
                }) + "\n")
                child.stdin.flush()
                message = json.loads(replies.get(timeout=20))
                assert message["jsonrpc"] == "2.0" and message["id"] == sequence, message
                assert "error" not in message, message
                assert message["result"]["resultType"] == "complete", message
                return message["result"]

            def tool(name, **arguments):
                result = request("tools/call", name=name, arguments=arguments)
                assert not result.get("isError"), result
                return result["structuredContent"]

            try:
                discovered = request("server/discover")
                assert "2026-07-28" in discovered["supportedVersions"], discovered
                names = {tool["name"] for tool in request("tools/list")["tools"]}
                assert {"create_art", "inspect_art", "open_workbench", "close_workbench"} <= names
                guide = request("resources/read", uri="dotmend://guides/editing")
                assert any("open_workbench" in item.get("text", "") for item in guide["contents"])
                created = tool("create_art", target={
                    "resource_id": "installation-check", "width": 2, "height": 2,
                    "palette": ["#000000", "#FFFFFF"], "transparent_index": 0,
                    "allowed_indices": [0, 1], "constraints_ref": None, "requirements": [],
                }, initial={"kind": "fill", "index": 1})
                art = tool("inspect_art", art_id=created["art_id"], include_indices=True)
                assert art["indices"] == [[1, 1], [1, 1]], art
                exported = tool("export_art", art_id=created["art_id"])
                assert exported["ok"], exported
                preview = next(uri for uri in exported["files"] if uri.endswith("preview.png"))
                assert request("resources/read", uri=preview)["contents"][0]["mimeType"] == "image/png"
                opened = tool("open_workbench", control_id="installation-check")
                assert opened["ok"], opened
                with urlopen(opened["instance"]["url"], timeout=10) as response:
                    assert "Dotmend" in response.read().decode("utf-8")
                closed = tool("close_workbench", control_id="installation-check",
                              workbench_id=opened["instance"]["workbench_id"])
                assert closed["ok"], closed
                child.stdin.close()
                assert child.wait(timeout=20) == 0
                reader.join(timeout=5)
                assert replies.empty(), "Unexpected output on MCP stdout"
            except BaseException:
                child.kill()
                child.wait(timeout=10)
                stderr.seek(0)
                sys.stderr.write(stderr.read())
                raise
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=10)
                reader.join(timeout=5)
                child.stdout.close()
    print("MCP discovery, tools, embedded guide, pixel storage, export, and managed UI passed.")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python scripts/check_mcp.py PATH_TO_DOTMEND")
    check(sys.argv[1])
