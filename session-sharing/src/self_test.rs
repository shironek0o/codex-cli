use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::{PrivateDirectory, Result, serve};

fn request(ws: &mut WebSocket<UnixStream>, id: u64, method: &str, params: Value) -> Result<Value> {
    ws.send(Message::Text(
        json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
            .to_string()
            .into(),
    ))?;
    loop {
        match ws.read()? {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(text.as_str())?;
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    return value
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "native protocol self-test returned an error".into());
                }
            }
            Message::Ping(_) | Message::Pong(_) => ws.flush()?,
            _ => return Err("native protocol self-test connection closed".into()),
        }
    }
}

pub(super) fn run(binary: &Path) -> Result<()> {
    let directory = PrivateDirectory::create()?;
    let home = directory.0.join("test-home");
    let rollouts = home.join("sessions/2026/11/17");
    fs::create_dir_all(&rollouts)?;
    fs::create_dir_all(home.join("other-workspace"))?;
    for (index, owner) in [(1, "A"), (2, "B"), (3, "A")] {
        let id = format!("00000000-0000-4000-8000-{index:012}");
        let cwd = if index == 3 {
            home.join("other-workspace")
        } else {
            home.clone()
        };
        let rows = [
            json!({"timestamp":"2026-11-17T00:00:00Z", "type":"session_meta", "payload":{"id":id,"session_id":id,"timestamp":"2026-11-17T00:00:00Z","cwd":cwd,"originator":"codex-tui","cli_version":"0.169.0","source":"cli","model_provider":"openai","account_scoped":true,"creator_user_id":owner,"creator_account_id":owner}}),
            json!({"timestamp":"2026-11-17T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Synthetic shared-history test","kind":"plain"}}),
        ];
        fs::write(
            rollouts.join(format!("rollout-2026-11-17T00-00-00-{id}.jsonl")),
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )?;
    }
    let upstream_path = directory.0.join("test-backend.sock");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(directory.0.join("self-test.log"))?;
    let mut native = Command::new(binary)
        .args(["app-server", "--listen"])
        .arg(format!("unix://{}", upstream_path.display()))
        .args(["--disable", "cli_resume_account_check"])
        .env("CODEX_HOME", &home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()?;
    let result = (|| -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !upstream_path.exists() {
            if native.try_wait()?.is_some() {
                return Err("native self-test backend stopped during startup".into());
            }
            if std::time::Instant::now() >= deadline {
                return Err("native self-test backend startup timed out".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        test_shared_connections(&directory, &upstream_path, &home)
    })();
    let _ = native.kill();
    let _ = native.wait();
    result?;
    println!(
        "PASS: workspace filtering, cross-account discovery, and two writable connections to the same shared writer."
    );
    Ok(())
}

fn test_shared_connections(
    directory: &PrivateDirectory,
    upstream_path: &Path,
    workspace: &Path,
) -> Result<()> {
    let (listener, socket_path) = directory.listen()?;
    listener.set_nonblocking(true)?;
    let running = Arc::new(AtomicBool::new(true));
    let server_running = running.clone();
    let upstream_path = upstream_path.to_owned();
    let server = thread::spawn(move || -> Result<()> {
        let mut workers = Vec::new();
        while server_running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let path = upstream_path.clone();
                    let running = server_running.clone();
                    workers.push(thread::spawn(move || serve(stream, &path, &running)));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(error.into()),
            }
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "self-test connection worker failed")??;
        }
        Ok(())
    });
    // Late backend events can still arrive after the last resume reply.
    let mut connections = Vec::new();
    let result = (|| -> Result<()> {
        for _ in 0..2 {
            let stream = UnixStream::connect(&socket_path)?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            let (mut ws, _) = tungstenite::client("ws://localhost/codex", stream)?;
            request(
                &mut ws,
                1,
                "initialize",
                json!({"clientInfo":{"name":"codex-tui","version":"1.1"},"capabilities":{"experimentalApi":true}}),
            )?;
            ws.send(Message::Text(
                json!({"jsonrpc":"2.0","method":"initialized"})
                    .to_string()
                    .into(),
            ))?;
            let listing = request(
                &mut ws,
                2,
                "thread/list",
                json!({"currentAccountOnly":true,"modelProviders":[],"sourceKinds":["cli","vscode"],"limit":100,"cwd":workspace}),
            )?;
            if listing.get("data").and_then(Value::as_array).map(Vec::len) != Some(2) {
                return Err("native self-test did not discover both synthetic accounts".into());
            }
            if listing["data"]
                .as_array()
                .is_none_or(|rows| rows.iter().any(|row| row["cwd"] != json!(workspace)))
            {
                return Err("native self-test leaked a session from a different workspace".into());
            }
            let all = request(
                &mut ws,
                4,
                "thread/list",
                json!({"currentAccountOnly":true,"modelProviders":[],"sourceKinds":["cli","vscode"],"limit":100}),
            )?;
            if all.get("data").and_then(Value::as_array).map(Vec::len) != Some(3) {
                return Err("native self-test could not explicitly list all workspaces".into());
            }
            let resumed = request(
                &mut ws,
                3,
                "thread/resume",
                json!({"threadId":"00000000-0000-4000-8000-000000000001","excludeTurns":true}),
            )?;
            if resumed["thread"]["id"] != "00000000-0000-4000-8000-000000000001" {
                return Err("native self-test resumed a different thread".into());
            }
            connections.push(ws);
        }
        Ok(())
    })();
    running.store(false, Ordering::Relaxed);
    let server_result = server.join().map_err(|_| "self-test server failed")?;
    drop(connections); // Keep clients connected until all forwarding workers have exited.
    result.map_err(|e| format!("checking shared sessions: {e}"))?;
    server_result.map_err(|e| format!("finishing the self-test proxy: {e}").into())
}
