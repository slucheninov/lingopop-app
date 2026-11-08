//! Exec-isolated AppKit materialization. The parent accepts at most one 64 KiB text frame.
use crate::{clipboard_observer::MAX_HISTORY_TEXT_BYTES, PlatformError};
use std::{
    io::{Read, Write},
    os::unix::io::AsRawFd,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const FLAG: &str = "--lingopop-clipboard-observation-helper-v1";
const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_FRAME: usize = MAX_HISTORY_TEXT_BYTES + 5;

/// Call at executable entry, before the UI runtime or any application initialization. A test
/// harness must opt in explicitly; re-executing a libtest binary is not a helper implementation.
pub fn dispatch() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return false;
    }
    let expected = args
        .next()
        .and_then(|a| a.to_str().and_then(|s| s.parse::<u64>().ok()));
    let name = args.next();
    if args.next().is_some() {
        return true;
    }
    if let Some(expected) = expected {
        let result = objc2::rc::autoreleasepool(|_| {
            let pb = match name.as_ref() {
                None => objc2_app_kit::NSPasteboard::generalPasteboard(),
                Some(name) => match name.to_str() {
                    Some(name) => objc2_app_kit::NSPasteboard::pasteboardWithName(
                        &objc2_foundation::NSString::from_str(name),
                    ),
                    None => return Err(PlatformError::ClipboardBusy),
                },
            };
            super::clipboard_observer::read_pasteboard(&pb, expected)
        });
        // Fixed error frame: never serialize native errors or clipboard content as diagnostics.
        let mut out = std::io::stdout().lock();
        let _ = match result {
            Ok(Some(text)) if text.len() <= MAX_HISTORY_TEXT_BYTES => out.write_all(
                &[
                    &[1u8][..],
                    &(text.len() as u32).to_le_bytes(),
                    text.as_bytes(),
                ]
                .concat(),
            ),
            Ok(_) => out.write_all(&[0]),
            Err(PlatformError::Unsupported("clipboard text encoding")) => out.write_all(&[2]),
            Err(_) => out.write_all(&[3]),
        };
    }
    true
}

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn busy<T>(_: T) -> PlatformError {
    PlatformError::ClipboardBusy
}

pub(crate) fn read(expected: u64, name: Option<&str>) -> Result<Option<String>, PlatformError> {
    let exe = std::env::current_exe().map_err(busy)?;
    let mut command = Command::new(exe);
    // Session pasteboard access uses the inherited bootstrap namespace, not environment
    // credentials. Retain only basic per-user Foundation paths/encoding, never provider keys
    // or dynamic-loader injection variables from the parent environment.
    command.env_clear();
    for key in ["HOME", "TMPDIR", "__CF_USER_TEXT_ENCODING"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.arg(FLAG).arg(expected.to_string());
    if let Some(name) = name {
        command.arg(name);
    }
    collect(&mut command, TIMEOUT)
}

fn collect(command: &mut Command, timeout: Duration) -> Result<Option<String>, PlatformError> {
    // Command uses exec/spawn; no Objective-C work runs in a post-fork pre-exec closure.
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(busy)?;
    collect_child(child, timeout)
}

fn collect_child(child: Child, timeout: Duration) -> Result<Option<String>, PlatformError> {
    let mut child = Reap(child);
    let mut pipe = child.0.stdout.take().ok_or(PlatformError::ClipboardBusy)?;
    let fd = pipe.as_raw_fd();
    // A single reader owns this pipe. Nonblocking reads avoid an unjoinable reader thread when
    // AppKit or a delayed provider hangs. No payload is carried in arguments or diagnostics.
    // SAFETY: fd belongs to the live pipe and F_GETFL requires no pointer arguments.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(PlatformError::ClipboardBusy);
    }
    // SAFETY: fd is still open and F_SETFL takes integer status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(PlatformError::ClipboardBusy);
    }
    let deadline = Instant::now() + timeout;
    let mut frame = Vec::with_capacity(MAX_FRAME);
    let mut eof = false;
    loop {
        let mut chunk = [0u8; 4096];
        match pipe.read(&mut chunk) {
            Ok(0) => eof = true,
            Ok(n) => {
                if frame.len() + n > MAX_FRAME {
                    return Err(PlatformError::ClipboardBusy);
                }
                frame.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(PlatformError::ClipboardBusy),
        }
        if eof {
            if let Some(status) = child.0.try_wait().map_err(busy)? {
                if !status.success() {
                    return Err(PlatformError::ClipboardBusy);
                }
                return decode(&frame);
            }
        }
        if Instant::now() >= deadline {
            return Err(PlatformError::ClipboardBusy);
        }
        if eof {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll points to one initialized pollfd for the duration of the call.
        unsafe {
            libc::poll(&mut poll, 1, 10);
        }
    }
}
fn decode(frame: &[u8]) -> Result<Option<String>, PlatformError> {
    if frame == [0] {
        return Ok(None);
    }
    if frame == [2] {
        return Err(PlatformError::Unsupported("clipboard text encoding"));
    }
    if frame.len() < 5 || frame[0] != 1 {
        return Err(PlatformError::ClipboardBusy);
    }
    let len = u32::from_le_bytes(frame[1..5].try_into().map_err(busy)?) as usize;
    if len > MAX_HISTORY_TEXT_BYTES || frame.len() != 5 + len {
        return Err(PlatformError::ClipboardBusy);
    }
    String::from_utf8(frame[5..].to_vec())
        .map(Some)
        .map_err(busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_is_strict_and_bounded() {
        assert!(decode(&[0]).unwrap().is_none());
        assert_eq!(
            decode(&[1, 2, 0, 0, 0, b'o', b'k']).unwrap().as_deref(),
            Some("ok")
        );
        for frame in [
            &[0, 0][..],
            &[2],
            &[1, 1, 0, 0, 0, 255],
            &[1, 0, 0, 0, 0, 0],
            &[1, 0, 0, 1, 0],
        ] {
            assert!(decode(frame).is_err());
        }
        let mut maximum = vec![1];
        maximum.extend_from_slice(&(MAX_HISTORY_TEXT_BYTES as u32).to_le_bytes());
        maximum.resize(MAX_FRAME, b'x');
        assert_eq!(
            decode(&maximum).unwrap().unwrap().len(),
            MAX_HISTORY_TEXT_BYTES
        );
        maximum.push(b'x');
        assert!(decode(&maximum).is_err());
    }
    #[test]
    fn crash_malformed_and_oversized_helpers_fail_closed() {
        for script in [
            "kill -KILL $$",
            "printf '\\001\\002'",
            "head -c 70000 /dev/zero",
        ] {
            assert!(collect(
                Command::new("/bin/sh").args(["-c", script]),
                Duration::from_secs(2)
            )
            .is_err());
        }
    }
    #[test]
    fn timeout_kills_and_reaps_direct_child() {
        let child = Command::new("/bin/sleep")
            .arg("60")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        assert!(collect_child(child, Duration::ZERO).is_err());
        // waitpid returning ECHILD proves this process already reaped the helper.
        let mut status = 0;
        // SAFETY: status is writable and pid identifies the child created above.
        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(waited, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
