use std::fs::{DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tungstenite::WebSocket;

use crate::Result;
use crate::quota_poll::{QuotaCheck, check_quota};
use crate::usage_handoff::{control_connection, lock_handoff, request, warn};

const RECHECK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GoalStamp {
    created: i64,
    updated: i64,
}

fn stopped_goal(reply: &Value) -> Option<GoalStamp> {
    let goal = &reply["goal"];
    if goal["status"] != "usageLimited" {
        return None;
    }
    if let Some(budget) = goal["tokenBudget"].as_i64()
        && goal["tokensUsed"].as_i64()? >= budget
    {
        return None;
    }
    Some(GoalStamp {
        created: goal["createdAt"].as_i64()?,
        updated: goal["updatedAt"].as_i64()?,
    })
}

fn read_goal(backend: &mut WebSocket<UnixStream>, thread: &str) -> Result<Option<GoalStamp>> {
    Ok(stopped_goal(&goal_details(backend, thread)?))
}

fn goal_details(backend: &mut WebSocket<UnixStream>, thread: &str) -> Result<Value> {
    request(backend, "thread/goal/get", json!({"threadId": thread}))
}

pub fn schedule_or_warn(
    client: &mut WebSocket<UnixStream>,
    upstream: &Path,
    thread: &str,
    reason: &str,
) -> Result<()> {
    let message = match schedule(upstream, thread) {
        Ok(true) => "Goal queued to resume when the current account has quota again.".to_owned(),
        Ok(false) => reason.to_owned(),
        Err(error) => format!("{reason} Could not schedule quota recovery: {error}."),
    };
    warn(client, Some(thread), &message)
}

fn schedule(upstream: &Path, thread: &str) -> Result<bool> {
    let mut backend = control_connection(upstream)?;
    let Some(goal) = read_goal(&mut backend, thread)? else {
        return Ok(false);
    };
    let status = Command::new("setsid")
        .arg("--fork")
        .arg(std::env::current_exe()?)
        .arg("--wait-for-goal-quota")
        .arg(upstream)
        .args([thread, &goal.created.to_string(), &goal.updated.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("could not start the account quota watcher".into());
    }
    Ok(true)
}

pub fn run(args: &[String]) -> Result<()> {
    let [socket, thread, created, updated] = args else {
        return Err("invalid Goal quota watcher arguments".into());
    };
    if thread.len() != 36
        || !thread
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("invalid Goal thread ID".into());
    }
    let mut stamp = GoalStamp {
        created: created.parse()?,
        updated: updated.parse()?,
    };
    let upstream = Path::new(socket);
    let home = upstream
        .parent()
        .and_then(Path::parent)
        .ok_or("invalid daemon socket path")?;
    let locks = home.join("session-sharing-goal-waits");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&locks)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(locks.join(format!("{thread}-{}.lock", stamp.created)))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let mut running = false;
    loop {
        match poll(upstream, home, thread, &mut stamp, &mut running) {
            Ok(None) => return Ok(()),
            Ok(Some(delay)) => thread::sleep(delay),
            Err(_) => thread::sleep(RECHECK),
        }
    }
}

// Keep watching the same Goal after recovery, including when the terminal has closed.
fn poll(
    upstream: &Path,
    home: &Path,
    thread_id: &str,
    stamp: &mut GoalStamp,
    running: &mut bool,
) -> Result<Option<Duration>> {
    let mut backend = control_connection(upstream)?;
    let details = goal_details(&mut backend, thread_id)?;
    if *running {
        let goal = &details["goal"];
        if goal["createdAt"].as_i64() != Some(stamp.created) {
            return Ok(None);
        }
        if goal["status"] == "active" {
            return Ok(Some(RECHECK));
        }
        let Some(new_stop) = stopped_goal(&details) else {
            return Ok(None);
        };
        *stamp = new_stop;
        *running = false;
    } else if stopped_goal(&details) != Some(*stamp) {
        return Ok(None);
    }
    if !thread_is_idle(&mut backend, thread_id)? {
        return Ok(Some(RECHECK));
    }
    let resumed = request(
        &mut backend,
        "thread/resume",
        json!({
            "threadId": thread_id, "excludeTurns": true,
        }),
    )?;
    if resumed["thread"]["id"] != thread_id {
        return Err("daemon did not attach to the quota-stopped thread".into());
    }
    let _handoff = lock_handoff(home)?;
    let quota = request(&mut backend, "account/rateLimits/read", json!({}))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    if let QuotaCheck::Wait(delay) = check_quota(&quota, resumed["model"].as_str(), now) {
        return Ok(Some(delay));
    }
    if read_goal(&mut backend, thread_id)? != Some(*stamp) {
        return Ok(None);
    }
    if !thread_is_idle(&mut backend, thread_id)? {
        return Ok(Some(RECHECK));
    }
    let active = request(
        &mut backend,
        "thread/goal/set",
        json!({
            "threadId": thread_id, "status": "active",
        }),
    )?;
    if active["goal"]["status"] != "active" {
        return Err("daemon did not reactivate the Goal".into());
    }
    *running = true;
    Ok(Some(RECHECK))
}

fn thread_is_idle(backend: &mut WebSocket<UnixStream>, thread: &str) -> Result<bool> {
    let reply = request(
        backend,
        "thread/read",
        json!({"threadId": thread, "includeTurns": false}),
    )?;
    Ok(matches!(
        reply["thread"]["status"]["type"].as_str(),
        Some("idle" | "systemError" | "notLoaded")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_only_for_the_same_quota_stop_with_budget_left() {
        let mut reply = json!({"goal": {
            "status": "usageLimited", "createdAt": 1, "updatedAt": 2,
            "tokensUsed": 100, "tokenBudget": 200,
        }});
        assert_eq!(
            stopped_goal(&reply),
            Some(GoalStamp {
                created: 1,
                updated: 2
            })
        );
        reply["goal"]["tokensUsed"] = json!(200);
        assert_eq!(stopped_goal(&reply), None);
        reply["goal"]["tokenBudget"] = Value::Null;
        reply["goal"]["status"] = json!("paused");
        assert_eq!(stopped_goal(&reply), None);
    }
}
