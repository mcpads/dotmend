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
            .env("DOTMEND_RUNTIME_DIR", project.join("runtime"))
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
        self.send(name, arguments);
        self.receive()
    }
    fn send(&mut self, name: &str, arguments: Value) {
        self.sequence += 1;
        let request = json!({"jsonrpc":"2.0","id":self.sequence,"method":"tools/call","params":{
            "name":name,"arguments":arguments,"_meta":{
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientCapabilities":{}
            }
        }});
        writeln!(self.child.stdin.as_mut().unwrap(), "{request}").unwrap();
    }
    fn receive(&mut self) -> Value {
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

#[test]
fn declared_work_preserves_long_calls_until_handoff_or_host_death() {
    for killed in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut host = Client::open(project.path());
        let mut worker = Client::open(project.path());
        assert_eq!(worker.call("list_art", json!({}))["ok"], true);
        let control = "active-tool-workbench-owner";
        let opened = host.call(
            "open_workbench",
            json!({"control_id":control,"idle_timeout_seconds":1,"work_state":"working"}),
        );
        assert_eq!(opened["ok"], true, "{opened}");
        let database =
            rusqlite::Connection::open(project.path().join(".dotmend/art.sqlite")).unwrap();
        database.execute_batch("BEGIN IMMEDIATE").unwrap();
        worker.send(
            "create_art",
            json!({"target":{
            "resource_id":"busy-art","width":1,"height":1,"palette":["#000000"],
            "transparent_index":null,"allowed_indices":[0],"constraints_ref":null,"requirements":[]
        },"initial":{"kind":"fill","index":0}}),
        );
        // A real tool waits on the SQLite write lock longer than the idle deadline.
        std::thread::sleep(std::time::Duration::from_millis(1600));
        let current = host.call("inspect_workbench", json!({"control_id":control}));
        if killed {
            worker.child.kill().unwrap();
            worker.child.wait().unwrap();
        }
        database.execute_batch("ROLLBACK").unwrap();
        if !killed {
            let result = worker.receive();
            assert_eq!(result["ok"], true, "{result}");
        }
        assert_eq!(current["state"], "owned", "{current}");
        assert_eq!(current["instance"], opened["instance"]);
        if killed {
            host.child.kill().unwrap();
            host.child.wait().unwrap();
            let mut replacement = Client::open(project.path());
            let reopened = replacement.call("open_workbench", json!({"control_id":control}));
            assert_eq!(reopened["state"], "owned");
            assert_ne!(
                reopened["instance"]["workbench_id"],
                opened["instance"]["workbench_id"]
            );
            assert_eq!(reopened["work_state"], "waiting");
        } else {
            assert_eq!(
                host.call(
                    "open_workbench",
                    json!({"control_id":control,"work_state":"waiting"})
                )["work_state"],
                "waiting"
            );
            std::thread::sleep(std::time::Duration::from_millis(1600));
            assert_eq!(
                host.call("inspect_workbench", json!({"control_id":control}))["state"],
                "closed"
            );
        }
    }
}

#[test]
fn project_storage_is_ignored_without_changing_existing_ignore_rules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let excludes = temp.path().join("global-ignore");
    std::fs::write(&excludes, "").unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q", "--template="])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join(".gitignore"), "/build/\n").unwrap();
    let ignored = |path: &str| {
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("-c")
            .arg(format!("core.excludesFile={}", excludes.display()))
            .args(["check-ignore", "-q", path])
            .status()
            .unwrap()
            .success()
    };
    let mut client = Client::open(&root);
    assert_eq!(client.call("list_art", json!({}))["ok"], true);
    for path in [
        ".dotmend/art.sqlite",
        ".dotmend/art.sqlite-wal",
        ".dotmend/exports/preview.png",
        ".dotmend/workbench.json",
        ".dotmend/.gitignore",
    ] {
        assert!(ignored(path), "Storage is exposed to Git: {path}");
    }
    assert!(!ignored("input.png"));
    assert_eq!(
        std::fs::read_to_string(root.join(".gitignore")).unwrap(),
        "/build/\n"
    );
    drop(client);
    let custom = "# Project choice\n*\n";
    std::fs::write(root.join(".dotmend/.gitignore"), custom).unwrap();
    let mut resumed = Client::open(&root);
    assert_eq!(resumed.call("list_art", json!({}))["ok"], true);
    assert_eq!(
        std::fs::read_to_string(root.join(".dotmend/.gitignore")).unwrap(),
        custom
    );
}
