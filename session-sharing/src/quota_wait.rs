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
use crate::quota_settings::{Setting, home_for_socket};
use crate::usage_handoff::{
    ResumeKind, continuation_params, control_connection, lock_handoff, quota_failed_turn, request,
    warn,
};

const RECHECK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GoalStamp {
    created: i64,
    updated: i64,
}

enum Work {
    Goal { stamp: GoalStamp, running: bool },
    Turn(String),
}

impl Work {
    fn is_pending(&self, backend: &mut WebSocket<UnixStream>, thread: &str) -> Result<bool> {
        match self {
            Self::Goal { stamp, .. } => Ok(read_goal(backend, thread)? == Some(*stamp)),
            Self::Turn(turn) => {
                let history = request(
                    backend,
                    "thread/turns/list",
                    json!({
                        "threadId": thread, "limit": 1, "sortDirection": "desc", "itemsView": "notLoaded",
                    }),
                )?;
                let latest = &history["data"][0];
                if latest["id"] != *turn || !quota_failed_turn(latest) {
                    return Ok(false);
                }
                let goal = goal_details(backend, thread)?;
                Ok(!matches!(
                    goal["goal"]["status"].as_str(),
                    Some("active" | "usageLimited")
                ))
            }
        }
    }
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
    kind: &ResumeKind,
    reason: &str,
) -> Result<()> {
    let message = match schedule(upstream, thread, kind) {
        Ok(Some(work)) => {
            format!("{work} queued to resume when the current account has quota again.")
        }
        Ok(None) => reason.to_owned(),
        Err(error) => format!("{reason} Could not schedule quota recovery: {error}."),
    };
    warn(client, Some(thread), &message)
}

fn schedule(upstream: &Path, thread: &str, kind: &ResumeKind) -> Result<Option<&'static str>> {
    if !Setting::Resume.enabled(home_for_socket(upstream)?) {
        return Ok(None);
    }
    let (work, flag, identity) = match kind {
        ResumeKind::Goal => {
            let mut backend = control_connection(upstream)?;
            let Some(goal) = read_goal(&mut backend, thread)? else {
                return Ok(None);
            };
            (
                "Goal",
                "--wait-for-goal-quota",
                vec![goal.created.to_string(), goal.updated.to_string()],
            )
        }
        ResumeKind::Turn(turn) if id_is_valid(turn) => {
            ("Turn", "--wait-for-turn-quota", vec![turn.clone()])
        }
        _ => return Ok(None),
    };
    let status = Command::new("setsid")
        .arg("--fork")
        .arg(std::env::current_exe()?)
        .arg(flag)
        .arg(upstream)
        .arg(thread)
        .args(identity)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("could not start the account quota watcher".into());
    }
    Ok(Some(work))
}

pub fn run(args: &[String]) -> Result<()> {
    let [flag, socket, thread, identity @ ..] = args else {
        return Err("invalid quota watcher arguments".into());
    };
    if !id_is_valid(thread) {
        return Err("invalid thread ID".into());
    }
    let (mut work, directory, lock_id) = match (flag.as_str(), identity) {
        ("--wait-for-goal-quota", [created, updated]) => {
            let stamp = GoalStamp {
                created: created.parse()?,
                updated: updated.parse()?,
            };
            (
                Work::Goal {
                    stamp,
                    running: false,
                },
                "session-sharing-goal-waits",
                stamp.created.to_string(),
            )
        }
        ("--wait-for-turn-quota", [turn]) if id_is_valid(turn) => (
            Work::Turn(turn.clone()),
            "session-sharing-turn-waits",
            turn.clone(),
        ),
        _ => return Err("invalid quota watcher arguments".into()),
    };
    let upstream = Path::new(socket);
    let home = home_for_socket(upstream)?;
    let locks = home.join(directory);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&locks)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(locks.join(format!("{thread}-{lock_id}.lock")))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    loop {
        match poll(upstream, home, thread, &mut work) {
            Ok(None) => return Ok(()),
            Ok(Some(delay)) => thread::sleep(delay),
            Err(_) => thread::sleep(RECHECK),
        }
    }
}

fn id_is_valid(id: &str) -> bool {
    id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

fn poll(
    upstream: &Path,
    home: &Path,
    thread_id: &str,
    work: &mut Work,
) -> Result<Option<Duration>> {
    let mut backend = control_connection(upstream)?;
    // Keep watching a running Goal for its next quota stop after recovery.
    if let Work::Goal {
        stamp,
        running: true,
    } = work
    {
        let details = goal_details(&mut backend, thread_id)?;
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
        *work = Work::Goal {
            stamp: new_stop,
            running: false,
        };
    } else if !work.is_pending(&mut backend, thread_id)? {
        return Ok(None);
    }
    if !Setting::Resume.enabled(home) || !thread_is_idle(&mut backend, thread_id)? {
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
    if !Setting::Resume.enabled(home) {
        return Ok(Some(RECHECK));
    }
    let quota = request(&mut backend, "account/rateLimits/read", json!({}))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    if let QuotaCheck::Wait(delay) = check_quota(&quota, resumed["model"].as_str(), now) {
        return Ok(Some(delay));
    }
    if !work.is_pending(&mut backend, thread_id)? {
        return Ok(None);
    }
    if !thread_is_idle(&mut backend, thread_id)? || !Setting::Resume.enabled(home) {
        return Ok(Some(RECHECK));
    }
    if let Work::Goal { running, .. } = work {
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
        return Ok(Some(RECHECK));
    }
    // A lost reply does not tell us whether the daemon already started the turn.
    if let Err(error) = request(&mut backend, "turn/start", continuation_params(thread_id)) {
        eprintln!("Codex did not confirm the continued turn: {error}");
    }
    Ok(None)
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
