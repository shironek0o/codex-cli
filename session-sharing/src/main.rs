use std::fs::{self, DirBuilder};
use std::io::{self, BufRead, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tungstenite::{Message, WebSocket};

mod self_test;
mod terminal_output;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    fn create() -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("codex-history-{}-{nonce:x}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn listen(&self) -> io::Result<(UnixListener, PathBuf)> {
        let path = self.0.join("rpc.sock");
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok((listener, path))
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Change only the account filter on local history requests.
fn share_listing(text: &str) -> Result<String> {
    let mut request: Value = serde_json::from_str(text)?;
    if matches!(
        request.get("method").and_then(Value::as_str),
        Some("thread/list" | "thread/search")
    ) {
        let params = request
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .ok_or("history request is missing its parameters")?;
        params.insert("currentAccountOnly".into(), Value::Bool(false));
        return Ok(serde_json::to_string(&request)?);
    }
    Ok(text.to_owned())
}

fn backend_options(args: &[String]) -> Vec<String> {
    let mut options = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-c" | "--config" | "--enable" | "--disable") {
            if let Some(value) = args.get(index + 1) {
                options.extend([arg.clone(), value.clone()]);
                index += 2;
                continue;
            }
        } else if ["--config=", "--enable=", "--disable="]
            .iter()
            .any(|prefix| arg.starts_with(prefix))
        {
            options.push(arg.clone());
        }
        index += 1;
    }
    options
}

fn needs_proxy(args: &[String]) -> bool {
    if std::env::var_os("CODEX_SESSION_SHARING_OFF").is_some() {
        return false;
    }
    if args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--remote" | "--no-daemon" | "-p" | "--profile"
        ) || arg.starts_with("--remote=")
            || arg.starts_with("--profile=")
    }) {
        return false;
    }
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if matches!(arg.as_str(), "--help" | "-h" | "--version" | "-V") {
            return false;
        }
        if arg == "--" {
            return true;
        }
        if matches!(
            arg.as_str(),
            "-c" | "--config"
                | "--enable"
                | "--disable"
                | "-C"
                | "--cd"
                | "-m"
                | "--model"
                | "-s"
                | "--sandbox"
                | "-a"
                | "--ask-for-approval"
                | "--add-dir"
                | "-i"
                | "--image"
        ) {
            index += 2;
            continue;
        }
        if !arg.starts_with('-') {
            return !matches!(
                arg.as_str(),
                "app-server"
                    | "exec"
                    | "e"
                    | "login"
                    | "logout"
                    | "completion"
                    | "features"
                    | "debug"
                    | "mcp"
                    | "mcp-server"
                    | "update"
                    | "cloud"
                    | "sandbox"
                    | "apply"
                    | "review"
                    | "help"
            );
        }
        index += 1;
    }
    true
}

// Older reconnect hints may refer to an expired adapter socket.
fn is_adapter_endpoint(endpoint: &str) -> bool {
    let Some(path) = endpoint.strip_prefix("unix://").map(Path::new) else {
        return false;
    };
    if path.file_name().and_then(|value| value.to_str()) != Some("rpc.sock") {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if parent.parent() != Some(std::env::temp_dir().as_path())
        && parent.parent() != Some(Path::new("/tmp"))
    {
        return false;
    }
    let Some(name) = parent
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_prefix("codex-history-"))
    else {
        return false;
    };
    let Some((pid, nonce)) = name.split_once('-') else {
        return false;
    };
    pid.parse::<u32>().is_ok()
        && !nonce.is_empty()
        && nonce.chars().all(|value| value.is_ascii_hexdigit())
}

fn normalize_reconnect_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            result.extend_from_slice(&args[index..]);
            break;
        }
        if argument == "--remote"
            && args
                .get(index + 1)
                .is_some_and(|endpoint| is_adapter_endpoint(endpoint))
        {
            index += 2;
            continue;
        }
        if argument
            .strip_prefix("--remote=")
            .is_some_and(is_adapter_endpoint)
        {
            index += 1;
            continue;
        }
        result.push(argument.clone());
        index += 1;
    }
    result
}

fn read_message(ws: &mut WebSocket<UnixStream>) -> Result<Option<Message>> {
    match ws.read() {
        Ok(message) => Ok(Some(message)),
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
            Ok(Some(Message::Close(None)))
        }
        Err(error) => Err(error.into()),
    }
}

// Route every connection to the same daemon so it retains the session writer.
fn serve(stream: UnixStream, upstream_path: &Path, running: &AtomicBool) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut client = tungstenite::accept(stream)?;
    let upstream = UnixStream::connect(upstream_path)?;
    upstream.set_read_timeout(Some(Duration::from_secs(10)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let (mut backend, _) = tungstenite::client("ws://localhost/codex", upstream)?;
    client
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(5)))?;
    backend
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(5)))?;
    let result = (|| -> Result<()> {
        while running.load(Ordering::Relaxed) {
            if let Some(message) = read_message(&mut client)? {
                match message {
                    Message::Text(text) => {
                        backend.send(Message::Text(share_listing(text.as_str())?.into()))?
                    }
                    Message::Close(_) => return Ok(()),
                    Message::Ping(_) | Message::Pong(_) => client.flush()?,
                    _ => return Err(
                        "the native app-server protocol changed; update the shared-history adapter"
                            .into(),
                    ),
                }
            }
            // Limit each batch so backend events cannot starve keyboard input.
            for _ in 0..32 {
                match read_message(&mut backend)? {
                    Some(Message::Text(text)) => client
                        .send(Message::Text(text))
                        .map_err(|e| format!("forwarding a backend event to the client: {e}"))?,
                    Some(Message::Close(_)) => return Ok(()),
                    Some(Message::Ping(_)) | Some(Message::Pong(_)) => backend.flush()?,
                    Some(_) => return Err(
                        "the native app-server protocol changed; update the shared-history adapter"
                            .into(),
                    ),
                    None => break,
                }
            }
        }
        Ok(())
    })();
    let _ = client.close(None);
    let _ = backend.close(None);
    result
}

fn shared_daemon_socket(binary: &Path, options: &[String]) -> Result<PathBuf> {
    // Starting an existing daemon reuses it and keeps active tasks running.
    let output = Command::new(binary)
        .args(options)
        .args(["app-server", "daemon", "start"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "could not start the shared Codex daemon: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or("cannot locate CODEX_HOME")?;
    Ok(home.join("app-server-control/app-server-control.sock"))
}

// Exercise the same request filter in environments without Unix sockets.
fn run_stdio(binary: &Path) -> Result<i32> {
    let mut backend = Command::new(binary)
        .args([
            "app-server",
            "--stdio",
            "--disable",
            "cli_resume_account_check",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut input = backend.stdin.take().ok_or("backend stdin is unavailable")?;
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let result = line
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })
                .and_then(|line| share_listing(&line));
            match result {
                Ok(line) => {
                    if writeln!(input, "{line}")
                        .and_then(|_| input.flush())
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("Shared history error: {error}");
                    break;
                }
            }
        }
    });
    let exit_status = backend.wait()?;
    Ok(exit_status.code().unwrap_or(1))
}

// The remote picker defaults to all directories unless given an explicit cwd.
fn scoped_client_args(args: &[String], cwd: &Path) -> Vec<String> {
    let mut result = args.to_vec();
    let mut found = false;
    let mut args = result.iter_mut();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if matches!(arg.as_str(), "-C" | "--cd") {
            found = true;
            if let Some(path) = args.next() {
                if !Path::new(path).is_absolute() {
                    *path = cwd.join(&*path).to_string_lossy().into_owned();
                }
            }
        } else if let Some(path) = arg
            .strip_prefix("--cd=")
            .or_else(|| arg.strip_prefix("-C").filter(|path| !path.is_empty()))
        {
            found = true;
            *arg = format!("--cd={}", cwd.join(path).display());
        }
    }
    if !found {
        result.splice(
            0..0,
            ["--cd".to_owned(), cwd.to_string_lossy().into_owned()],
        );
    }
    result
}

fn run_client(binary: &Path, args: &[String]) -> Result<i32> {
    let upstream_path = shared_daemon_socket(binary, &backend_options(args))?;
    let directory = PrivateDirectory::create()?;
    let (listener, socket_path) = directory.listen()?;
    listener.set_nonblocking(true)?;
    let running = Arc::new(AtomicBool::new(true));
    let endpoint = format!("unix://{}", socket_path.display());
    let mut output = terminal_output::TerminalOutput::prepare(&endpoint)?;
    let mut command = Command::new(binary);
    command
        .arg("--remote")
        .arg(&endpoint)
        .args(scoped_client_args(args, &std::env::current_dir()?));
    if let Some(output) = &output {
        output.configure(&mut command)?;
    }
    let mut client = command.spawn()?;
    if let Some(output) = &mut output {
        output.start();
    }
    let mut workers = Vec::new();
    let exit_status = loop {
        if let Some(exit_status) = client.try_wait()? {
            break exit_status;
        }
        if let Some(output) = &mut output {
            output.resize(&client);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let upstream_path = upstream_path.clone();
                let running = running.clone();
                workers.push(thread::spawn(move || {
                    if let Err(error) = serve(stream, &upstream_path, &running) {
                        eprintln!("Shared history error: {error}");
                    }
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(error) => {
                let _ = client.kill();
                let _ = client.wait();
                running.store(false, Ordering::Relaxed);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(error.into());
            }
        }
    };
    running.store(false, Ordering::Relaxed);
    for worker in workers {
        let _ = worker.join();
    }
    if let Some(output) = &mut output {
        output.finish()?;
    }
    Ok(exit_status
        .code()
        .unwrap_or_else(|| 128 + exit_status.signal().unwrap_or(1)))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("codex-session-sharing {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let self_test_mode = args.first().is_some_and(|arg| arg == "--self-test");
    let stdio_mode = args.first().is_some_and(|arg| arg == "--stdio-filter");
    if self_test_mode || stdio_mode {
        args.remove(0);
    }
    if args.len() < 2 || args[0] != "--binary" {
        eprintln!("Usage: codex-session-sharing [--self-test] --binary CODEX [-- CLI_ARGS...]");
        std::process::exit(2);
    }
    let binary = PathBuf::from(&args[1]);
    args.drain(..2);
    if args.first().is_some_and(|arg| arg == "--") {
        args.remove(0);
    }
    args = normalize_reconnect_args(&args);
    if !self_test_mode && !stdio_mode && !needs_proxy(&args) {
        let error = Command::new(binary).args(args).exec();
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
    let result = if self_test_mode {
        self_test::run(&binary).map(|_| 0)
    } else if stdio_mode {
        run_stdio(&binary)
    } else {
        run_client(&binary, &args)
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Shared history error: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expired_adapter_hints_reconnect_without_changing_real_remote_targets() {
        let endpoint = "unix:///tmp/codex-history-498447-18e8e1361b0775b2/rpc.sock";
        assert!(is_adapter_endpoint(endpoint));
        assert_eq!(
            normalize_reconnect_args(&[
                "--remote".into(),
                endpoint.into(),
                "resume".into(),
                "thread-id".into()
            ]),
            ["resume", "thread-id"]
        );
        assert_eq!(
            normalize_reconnect_args(&[format!("--remote={endpoint}"), "agents".into()]),
            ["agents"]
        );
        for remote in [
            "wss://example.com/",
            "unix:///tmp/real-server.sock",
            "unix:///tmp/codex-history-invalid-id/rpc.sock",
            "unix:///other/codex-history-123-abc/rpc.sock",
        ] {
            let args = vec!["--remote".into(), remote.into(), "resume".into()];
            assert_eq!(normalize_reconnect_args(&args), args);
        }
        let prompt = vec!["--".into(), "--remote".into(), endpoint.into()];
        assert_eq!(normalize_reconnect_args(&prompt), prompt);
    }

    #[test]
    fn workspace_defaults_and_explicit_all_survive_remote_launch() {
        let cwd = Path::new("/local/project");
        assert_eq!(
            scoped_client_args(&["resume".into()], cwd),
            ["--cd", "/local/project", "resume"]
        );
        assert_eq!(
            scoped_client_args(&["resume".into(), "--all".into()], cwd),
            ["--cd", "/local/project", "resume", "--all"]
        );
        assert_eq!(
            scoped_client_args(&["resume".into(), "-C".into(), "other space".into()], cwd),
            ["resume", "-C", "/local/project/other space"]
        );
        assert_eq!(
            scoped_client_args(&["--cd=/explicit".into(), "resume".into()], cwd),
            ["--cd=/explicit", "resume"]
        );
    }

    #[test]
    fn preserves_history_filters_and_ordinary_requests() {
        let input = r#"{"id":7,"method":"thread/list","params":{"currentAccountOnly":true,"cwd":"/project","archived":true,"cursor":"next"}}"#;
        let output: Value = serde_json::from_str(&share_listing(input).unwrap()).unwrap();
        assert_eq!(
            output["params"],
            json!({"currentAccountOnly":false,"cwd":"/project","archived":true,"cursor":"next"})
        );
        let turn = r#"{"method":"turn/start","params":{"text":"currentAccountOnly"}}"#;
        assert_eq!(share_listing(turn).unwrap(), turn);
        assert!(share_listing(r#"{"method":"thread/list","params":null}"#).is_err());
    }

    #[test]
    fn passthrough_and_backend_options_preserve_cli_intent() {
        for command in [
            "exec",
            "app-server",
            "login",
            "logout",
            "features",
            "--version",
            "--help",
        ] {
            assert!(!needs_proxy(&[command.into()]));
        }
        assert!(needs_proxy(&["resume".into(), "--all".into()]));
        assert!(needs_proxy(&[]));
        let args: Vec<String> = ["-c", "model=example", "--disable", "apps", "resume"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(backend_options(&args), args[..4]);
    }
}
