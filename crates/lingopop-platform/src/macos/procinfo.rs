//! The kernel process start time: the identity of a process that survives pid reuse.
//!
//! `proc_pidinfo(pid, PROC_PIDTBSDINFO, ..)` fills a `struct proc_bsdinfo`, whose
//! `pbi_start_tvsec` / `pbi_start_tvusec` are the process start time (checked against the macOS
//! SDK headers `libproc.h` and `sys/proc_info.h`; the `libc` crate mirrors that layout). Unlike
//! `NSRunningApplication.launchDate` it exists for every process, including ones not launched
//! through LaunchServices.

use std::mem::{size_of, MaybeUninit};

/// `sec`/`usec` as microseconds; zero (unset) or an overflow is "unreadable".
fn micros(sec: u64, usec: u64) -> Option<i64> {
    if sec == 0 && usec == 0 {
        return None;
    }
    i64::try_from(sec)
        .ok()?
        .checked_mul(1_000_000)?
        .checked_add(i64::try_from(usec).ok()?)
}

/// Start time of `pid` in microseconds since the Unix epoch, `None` when the kernel will not say
/// (process gone, not ours to inspect, short read).
pub(crate) fn start_time_micros(pid: i32) -> Option<i64> {
    if pid <= 0 {
        return None;
    }
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a writable, zero-initialized buffer of exactly `size` bytes, and
    // PROC_PIDTBSDINFO fills a `proc_bsdinfo`; the kernel writes at most `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast::<libc::c_void>(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: the kernel reported a full `proc_bsdinfo`, and an all-zero value is valid for it
    // (plain integers and byte arrays).
    let info = unsafe { info.assume_init() };
    if i64::from(info.pbi_pid) != i64::from(pid) {
        return None;
    }
    micros(info.pbi_start_tvsec, info.pbi_start_tvusec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_combines_and_rejects_unset_or_overflowing_values() {
        assert_eq!(micros(1, 500_000), Some(1_500_000));
        assert_eq!(micros(0, 5), Some(5));
        assert_eq!(micros(0, 0), None);
        assert_eq!(micros(u64::MAX, 0), None);
        assert_eq!(micros(i64::MAX as u64, 0), None);
    }

    #[test]
    fn own_process_has_a_stable_start_time_and_bad_pids_have_none() {
        let own = std::process::id() as i32;
        let first = start_time_micros(own).expect("own process is inspectable");
        assert!(first > 0);
        assert_eq!(start_time_micros(own), Some(first));
        assert_eq!(start_time_micros(0), None);
        assert_eq!(start_time_micros(-1), None);
        assert_eq!(start_time_micros(i32::MAX), None);
    }
}
