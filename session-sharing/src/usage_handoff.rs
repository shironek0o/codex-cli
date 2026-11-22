use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::{Result, connect_backend};

const REQUEST_ID: &str = "codex-session-sharing/continue";

#[derive(Default)]
pub struct Handoff(Mutex<Option<(String, Instant)>>);

impl Handoff {
    pub fn remember(&self, thread_id: String) {
        *self.0.lock().unwrap() = Some((thread_id, Instant::now()));
    }

    pub fn take(&self, thread_id: &str) -> bool {
        let mut pending = self.0.lock().unwrap();
        if pending
            .as_ref()
            .is_some_and(|(id, since)| id == thread_id && since.elapsed() < Duration::from_secs(60))
        {
            pending.take();
            return true;
        }
        false
    }
}

enum Request {
    Start(String),
    Resume(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Quota(String),
    Resumed(String),
    ContinuationReply(bool),
    Forward,
}

#[derive(Default)]
pub struct Connection {
    requests: HashMap<String, Request>,
    submitted: HashMap<String, String>,
    early_completion: HashMap<String, (String, bool)>,
}

impl Connection {
    pub fn observe_request(&mut self, text: &str, handoff: &Handoff) {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let (Some(id), Some(thread)) = (message.get("id"), message["params"]["threadId"].as_str())
        else {
            return;
        };
        let request = match message["method"].as_str() {
            Some("turn/start") => {
                handoff.take(thread);
                Request::Start(thread.into())
            }
            Some("thread/resume") => Request::Resume(thread.into()),
            _ => return,
        };
        self.requests.insert(id.to_string(), request);
    }

    pub fn observe_event(&mut self, text: &str) -> Event {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return Event::Forward;
        };
        if message["method"] == "turn/completed" {
            let params = &message["params"];
            let turn = &params["turn"];
            let (Some(thread), Some(id)) = (params["threadId"].as_str(), turn["id"].as_str())
            else {
                return Event::Forward;
            };
            let quota = turn["status"] == "failed"
                && turn["error"]["codexErrorInfo"] == "usageLimitExceeded";
            if let Some(submitted) = self.submitted.remove(id) {
                if quota && submitted == thread {
                    return Event::Quota(thread.into());
                }
            } else if self
                .requests
                .values()
                .any(|request| matches!(request, Request::Start(pending) if pending == thread))
            {
                self.early_completion
                    .insert(id.into(), (thread.into(), quota));
            }
            return Event::Forward;
        }
        let Some(id) = message.get("id") else {
            return Event::Forward;
        };
        if message.get("method").is_some() {
            return Event::Forward;
        }
        if id == REQUEST_ID {
            return Event::ContinuationReply(message["result"]["turn"]["id"].is_string());
        }
        match self.requests.remove(&id.to_string()) {
            Some(Request::Start(thread)) => {
                if let Some(turn) = message["result"]["turn"]["id"].as_str() {
                    match self.early_completion.remove(turn) {
                        Some((completed, quota)) if completed == thread => {
                            if quota {
                                return Event::Quota(thread);
                            }
                        }
                        _ => {
                            self.submitted.insert(turn.into(), thread);
                        }
                    }
                }
            }
            Some(Request::Resume(thread)) if message["result"]["thread"]["id"] == thread => {
                return Event::Resumed(thread);
            }
            _ => {}
        }
        Event::Forward
    }
}

pub fn resume(backend: &mut WebSocket<UnixStream>, thread: &str) -> Result<()> {
    backend.send(rpc_request(REQUEST_ID, "turn/start", json!({
        "threadId": thread,
        "input": [{
            "type": "text",
            "text": "Continue the unfinished task from the previous turn. Check what was already done before repeating any action.",
            "text_elements": [],
        }],
    })))?;
    Ok(())
}

pub fn warn(client: &mut WebSocket<UnixStream>, thread: Option<&str>, message: &str) -> Result<()> {
    client.send(Message::Text(
        json!({
            "method": "warning",
            "params": {"threadId": thread, "message": message},
        })
        .to_string()
        .into(),
    ))?;
    Ok(())
}

pub enum Switch {
    Reloaded,
    Restarted,
}

pub fn switch_account(upstream: &Path, failed_thread: &str) -> Result<Switch> {
    let home = upstream
        .parent()
        .and_then(Path::parent)
        .ok_or("invalid daemon socket path")?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(home.join("session-sharing-handoff.lock"))?;
    lock.try_lock()
        .map_err(|_| "another account handoff is in progress")?;
    let accounts = auth_command(
        home,
        &["list", "--api", "--json"],
        "could not refresh saved accounts",
    )?;
    let target =
        eligible_account(&accounts).ok_or("no other saved ChatGPT account has confirmed quota")?;
    check_idle(upstream, failed_thread)?;
    let reply = auth_command(
        home,
        &["switch", target, "--json"],
        "codex-auth could not complete the account switch",
    )?;
    if reply["switched_to"]["account_key"] != target {
        return Err("codex-auth did not confirm the selected account".into());
    }
    match reply["daemon"].as_str() {
        Some("reloaded") => Ok(Switch::Reloaded),
        Some("restarted") => Ok(Switch::Restarted),
        _ => Err("the daemon did not confirm that it loaded the new account".into()),
    }
}

fn auth_command(home: &Path, args: &[&str], failure: &'static str) -> Result<Value> {
    let output = Command::new("timeout")
        .args(["--kill-after=2s", "135s", "codex-auth"])
        .args(args)
        .env("CODEX_HOME", home)
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(failure.into());
    }
    let reply: Value = serde_json::from_slice(&output.stdout).map_err(|_| failure)?;
    if reply["schema_version"] != 1 {
        return Err("unsupported codex-auth JSON format".into());
    }
    Ok(reply)
}

fn eligible_account(list: &Value) -> Option<&str> {
    let active = list["active_account_key"].as_str()?;
    let accounts = list["accounts"].as_array()?;
    accounts
        .iter()
        .find(|account| account["account_key"] == active && account["auth_mode"] == "chatgpt")?;
    accounts.iter().find_map(|account| {
        let key = account["account_key"].as_str()?;
        let usage = &account["usage"];
        if key == active || account["auth_mode"] != "chatgpt" || usage["refresh"]["status"] != "ok"
        {
            return None;
        }
        let windows = [&usage["primary"], &usage["secondary"]];
        let mut observed = false;
        for window in windows.into_iter().filter(|value| !value.is_null()) {
            let used = window["used_percent"].as_f64()?;
            if !(0.0..100.0).contains(&used) {
                return None;
            }
            observed = true;
        }
        observed.then_some(key)
    })
}

fn check_idle(upstream: &Path, failed_thread: &str) -> Result<()> {
    let mut backend = connect_backend(upstream)?;
    request(
        &mut backend,
        "initialize",
        json!({"clientInfo": {
            "name": "codex-session-sharing", "version": env!("CARGO_PKG_VERSION"),
        }}),
    )?;
    backend.send(Message::Text(
        json!({"method": "initialized"}).to_string().into(),
    ))?;
    let loaded = request(&mut backend, "thread/loaded/list", json!({}))?;
    let threads = loaded["data"]
        .as_array()
        .ok_or("could not check other Codex sessions")?;
    if !loaded["nextCursor"].is_null() || !threads.iter().any(|id| id == failed_thread) {
        return Err("the failed session is no longer owned by this daemon".into());
    }
    for id in threads.iter().filter(|id| *id != failed_thread) {
        let thread = request(
            &mut backend,
            "thread/read",
            json!({"threadId": id, "includeTurns": false}),
        )?;
        if !matches!(
            thread["thread"]["status"]["type"].as_str(),
            Some("idle" | "notLoaded" | "systemError")
        ) {
            return Err("another Codex session is still running".into());
        }
    }
    Ok(())
}

fn request(backend: &mut WebSocket<UnixStream>, method: &str, params: Value) -> Result<Value> {
    backend.send(rpc_request("check", method, params))?;
    loop {
        match backend.read()? {
            Message::Text(text) => {
                let message: Value = serde_json::from_str(text.as_str())?;
                if message["id"] == "check" && message.get("method").is_none() {
                    return message
                        .get("result")
                        .cloned()
                        .ok_or("could not check the shared Codex daemon".into());
                }
            }
            Message::Ping(_) => backend.flush()?,
            Message::Close(_) => return Err("the shared Codex daemon disconnected".into()),
            _ => {}
        }
    }
}

fn rpc_request(id: &str, method: &str, params: Value) -> Message {
    Message::Text(
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
            .to_string()
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = r#"{"id":7,"method":"turn/start","params":{"threadId":"thread"}}"#;
    const STARTED: &str = r#"{"id":7,"result":{"turn":{"id":"turn"}}}"#;
    const QUOTA: &str = r#"{"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"failed","error":{"codexErrorInfo":"usageLimitExceeded"}}}}"#;

    #[test]
    fn only_the_client_that_submitted_a_turn_can_recover_it_once() {
        let mut connection = Connection::default();
        let handoff = Handoff::default();
        assert_eq!(connection.observe_event(QUOTA), Event::Forward);
        connection.observe_request(START, &handoff);
        assert_eq!(connection.observe_event(STARTED), Event::Forward);
        assert_eq!(
            connection.observe_event(QUOTA),
            Event::Quota("thread".into())
        );
        assert_eq!(connection.observe_event(QUOTA), Event::Forward);
    }

    #[test]
    fn correlates_completion_even_if_it_arrives_before_the_start_reply() {
        let mut connection = Connection::default();
        connection.observe_request(START, &Handoff::default());
        assert_eq!(connection.observe_event(QUOTA), Event::Forward);
        assert_eq!(
            connection.observe_event(STARTED),
            Event::Quota("thread".into())
        );
    }

    #[test]
    fn retries_neither_transient_errors_nor_the_automatic_turn() {
        let mut connection = Connection::default();
        connection.observe_request(START, &Handoff::default());
        assert_eq!(connection.observe_event(STARTED), Event::Forward);
        let rate_limit = QUOTA.replace("usageLimitExceeded", "rateLimitExceeded");
        assert_eq!(connection.observe_event(&rate_limit), Event::Forward);
        let automatic_reply = STARTED.replace("7", &format!("\"{REQUEST_ID}\""));
        assert_eq!(
            connection.observe_event(&automatic_reply),
            Event::ContinuationReply(true)
        );
        assert_eq!(connection.observe_event(QUOTA), Event::Forward);
    }

    #[test]
    fn resumes_only_the_intended_thread_and_yields_to_user_input() {
        let handoff = Handoff::default();
        let mut connection = Connection::default();
        handoff.remember("thread".into());
        connection.observe_request(
            r#"{"id":8,"method":"thread/resume","params":{"threadId":"thread"}}"#,
            &handoff,
        );
        assert_eq!(
            connection.observe_event(r#"{"id":8,"result":{"thread":{"id":"thread"}}}"#),
            Event::Resumed("thread".into())
        );
        assert!(!handoff.take("other"));
        assert!(handoff.take("thread"));
        assert!(!handoff.take("thread"));
        handoff.remember("thread".into());
        connection.observe_request(START, &handoff);
        assert!(!handoff.take("thread"));
    }

    #[test]
    fn skips_stale_and_exhausted_accounts() {
        let account = |id, status, primary, secondary| {
            json!({
                "account_key": id,
                "auth_mode": "chatgpt",
                "usage": {
                    "refresh": {"status": status},
                    "primary": {"used_percent": primary},
                    "secondary": {"used_percent": secondary},
                },
            })
        };
        let mut list = json!({"active_account_key": "current", "accounts": [
            account("current", "ok", 1, 1),
            account("stale", "http_error", 1, 1),
            account("exhausted", "ok", 1, 100),
            account("available", "ok", 99, 50),
        ]});
        assert_eq!(eligible_account(&list), Some("available"));
        list["accounts"].as_array_mut().unwrap().pop();
        assert_eq!(eligible_account(&list), None);
    }
}
