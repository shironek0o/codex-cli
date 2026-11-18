use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

// Strip only the current proxy argument, preserving the UUID and terminal escapes.
struct OutputFilter {
    needle: Vec<u8>,
    pending: Vec<u8>,
}

impl OutputFilter {
    fn new(endpoint: &str) -> Self {
        let quoted = shlex::try_quote(endpoint).expect("local endpoint cannot contain NUL");
        Self {
            needle: format!("--remote {quoted} ").into_bytes(),
            pending: Vec::new(),
        }
    }

    fn push(&mut self, input: &[u8], output: &mut impl Write) -> io::Result<()> {
        self.pending.extend_from_slice(input);
        let mut offset = 0;
        while let Some(position) = self.pending[offset..]
            .windows(self.needle.len())
            .position(|bytes| bytes == self.needle)
        {
            output.write_all(&self.pending[offset..offset + position])?;
            offset += position + self.needle.len();
        }
        let remainder = &self.pending[offset..];
        let retained = (1..=remainder.len().min(self.needle.len() - 1))
            .rev()
            .find(|&length| {
                self.needle
                    .starts_with(&remainder[remainder.len() - length..])
            })
            .unwrap_or(0);
        output.write_all(&remainder[..remainder.len() - retained])?;
        self.pending = remainder[remainder.len() - retained..].to_vec();
        output.flush()
    }

    fn finish(&mut self, output: &mut impl Write) -> io::Result<()> {
        output.write_all(&self.pending)?;
        self.pending.clear();
        output.flush()
    }
}

fn dimensions(fd: RawFd) -> libc::winsize {
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let mut actual = std::mem::MaybeUninit::<libc::winsize>::uninit();
    // Read the kernel-provided dimensions only if ioctl succeeds.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, actual.as_mut_ptr()) } == 0 {
        let actual = unsafe { actual.assume_init() };
        if actual.ws_row > 0 && actual.ws_col > 0 {
            size = actual;
        }
    }
    size
}

pub struct TerminalOutput {
    master: Option<File>,
    slave: Option<File>,
    source_fd: RawFd,
    size: libc::winsize,
    endpoint: String,
    done: Arc<AtomicBool>,
    reader: Option<JoinHandle<io::Result<()>>>,
}

impl TerminalOutput {
    pub fn prepare(endpoint: &str) -> io::Result<Option<Self>> {
        if !io::stdout().is_terminal() {
            return Ok(None);
        }
        Self::create(endpoint, io::stdout().as_raw_fd()).map(Some)
    }

    fn create(endpoint: &str, source_fd: RawFd) -> io::Result<Self> {
        let size = dimensions(source_fd);
        let (mut master, mut slave) = (-1, -1);
        // Only stdout needs a PTY. Keep stdin, stderr, and the controlling terminal attached.
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(slave.as_raw_fd(), attributes.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = unsafe { attributes.assume_init() };
        unsafe { libc::cfmakeraw(&mut attributes) };
        if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            master: Some(master),
            slave: Some(slave),
            source_fd,
            size,
            endpoint: endpoint.into(),
            done: Arc::new(AtomicBool::new(false)),
            reader: None,
        })
    }

    pub fn configure(&self, command: &mut Command) -> io::Result<()> {
        command.stdout(Stdio::from(
            self.slave.as_ref().expect("PTY is open").try_clone()?,
        ));
        Ok(())
    }

    pub fn start(&mut self) {
        self.start_with_output(io::stdout());
    }

    fn start_with_output(&mut self, mut output: impl Write + Send + 'static) {
        let mut master = self.master.take().expect("PTY reader starts once");
        let mut filter = OutputFilter::new(&self.endpoint);
        let done = self.done.clone();
        self.reader = Some(thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                let mut ready = libc::pollfd {
                    fd: master.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let timeout = if done.load(Ordering::Acquire) { 0 } else { 100 };
                let count = unsafe { libc::poll(&mut ready, 1, timeout) };
                if count < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if count == 0 {
                    if done.load(Ordering::Acquire) {
                        break;
                    }
                    continue;
                }
                match master.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => filter.push(&buffer[..length], &mut output)?,
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            filter.finish(&mut output)
        }));
    }

    pub fn resize(&mut self, child: &Child) {
        let next = dimensions(self.source_fd);
        if (next.ws_row, next.ws_col, next.ws_xpixel, next.ws_ypixel)
            == (
                self.size.ws_row,
                self.size.ws_col,
                self.size.ws_xpixel,
                self.size.ws_ypixel,
            )
        {
            return;
        }
        if let Some(slave) = &self.slave {
            if unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSWINSZ, &next) } == 0 {
                self.size = next;
                // Notify the child after synchronizing the output PTY with its controlling terminal.
                unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGWINCH) };
            }
        }
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.done.store(true, Ordering::Release);
        self.slave.take();
        if let Some(reader) = self.reader.take() {
            return reader
                .join()
                .map_err(|_| io::Error::other("terminal output worker failed"))?;
        }
        Ok(())
    }
}

impl Drop for TerminalOutput {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const ENDPOINT: &str = "unix:///tmp/codex-history-123-abc/rpc.sock";
    const ID: &str = "01a1ea4a-0630-7950-8f9f-58e1b8a0c1f7";

    #[test]
    fn removes_only_own_remote_argument_at_every_chunk_boundary() {
        let input = format!(
            "\x1b[?1049l\r\n中文\r\n  \x1b[36mcodex --remote {ENDPOINT} resume {ID}\x1b[39m\r\nStop: codex --remote {ENDPOINT} agents\r\nOther: codex --remote wss://example.com/ resume other\r\n"
        );
        let expected = input.replace(&format!("--remote {ENDPOINT} "), "");
        for split in 0..=input.len() {
            let mut filter = OutputFilter::new(ENDPOINT);
            let mut output = Vec::new();
            filter
                .push(&input.as_bytes()[..split], &mut output)
                .unwrap();
            filter
                .push(&input.as_bytes()[split..], &mut output)
                .unwrap();
            filter.finish(&mut output).unwrap();
            assert_eq!(output, expected.as_bytes());
        }
    }

    #[test]
    fn preserves_incomplete_patterns_and_quotes_like_native_cli() {
        let endpoint = "unix:///tmp/path with space/rpc.sock";
        let input = format!(
            "codex --remote {} resume {ID}\n--rem",
            shlex::try_quote(endpoint).unwrap()
        );
        let mut output = Vec::new();
        let mut filter = OutputFilter::new(endpoint);
        for byte in input.as_bytes() {
            filter.push(&[*byte], &mut output).unwrap();
        }
        filter.finish(&mut output).unwrap();
        assert_eq!(output, format!("codex resume {ID}\n--rem").as_bytes());
    }

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn child_retains_terminal_stdout_and_live_output_is_filtered() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut output = TerminalOutput::create(ENDPOINT, -1).unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &format!("test -t 1 || exit 7; printf '\\033[32mReady\\033[0m\\n'; printf 'codex --remote {ENDPOINT} resume {ID}\\n'")]);
        output.configure(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        output.start_with_output(Capture(captured.clone()));
        assert!(child.wait().unwrap().success());
        output.finish().unwrap();
        assert_eq!(
            *captured.lock().unwrap(),
            format!("\x1b[32mReady\x1b[0m\ncodex resume {ID}\n").as_bytes()
        );
    }
}
