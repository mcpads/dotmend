#![cfg(feature = "native")]

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
};

struct Client {
    child: Child,
    output: BufReader<ChildStdout>,
    sequence: u64,
}
impl Client {
    fn open(project: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dotmend"))
            .current_dir(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            output,
            sequence: 0,
        }
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.sequence += 1;
        let request = json!({"jsonrpc":"2.0","id":self.sequence,"method":"tools/call","params":{
            "name":name,"arguments":arguments,"_meta":{
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientCapabilities":{}
            }
        }});
        writeln!(self.child.stdin.as_mut().unwrap(), "{request}").unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], self.sequence);
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"]["structuredContent"].clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.child.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn launch_directory_isolates_projects_and_shares_records_on_reconnect() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut a = Client::open(first.path());
    let mut b = Client::open(second.path());
    let created = a.call(
        "create_art",
        json!({"target":{
        "resource_id":"project-icon","width":1,"height":1,"palette":["#000000"],
        "transparent_index":null,"allowed_indices":[0],"constraints_ref":null,"requirements":[]
    },"initial":{"kind":"fill","index":0}}),
    );
    assert_eq!(created["ok"], true, "{created}");
    let id = created["art_id"].clone();
    assert!(
        b.call("list_art", json!({}))["arts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(b.call("inspect_art", json!({"art_id":id}))["ok"], false);
    assert!(first.path().join(".dotmend/art.sqlite").is_file());
    assert!(second.path().join(".dotmend/art.sqlite").is_file());
    assert!(!first.path().join(".retro-art").exists());
    let mut shared = Client::open(first.path());
    assert_eq!(shared.call("list_art", json!({}))["arts"][0]["art_id"], id);
    drop(a);
    assert_eq!(shared.call("inspect_art", json!({"art_id":id}))["ok"], true);
    drop(shared);
    let mut resumed = Client::open(first.path());
    assert_eq!(resumed.call("list_art", json!({}))["arts"][0]["art_id"], id);
}
