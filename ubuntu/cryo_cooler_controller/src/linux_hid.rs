//! Linux hidraw transport for the two Thermaltake telemetry interfaces.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
};

pub(crate) type Handle = File;

pub(crate) fn open(pid: u16, interface: &[u8]) -> Option<Handle> {
    let id = format!("HID_ID=0003:0000264A:{pid:08X}");
    fs::read_dir("/sys/class/hidraw").ok()?.flatten().find_map(|entry| {
        let device = entry.path().join("device");
        let event = fs::read_to_string(device.join("uevent")).ok()?;
        if !event.lines().any(|line| line == id)
            || !fs::read(device.join("report_descriptor")).ok()?.starts_with(interface)
        {
            return None;
        }
        let handle = OpenOptions::new().read(true).write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(std::path::Path::new("/dev").join(entry.file_name())).ok()?;
        // Recheck through the opened node in case a device was just replugged.
        let mut info = [0u8; 8];
        let status = unsafe { libc::ioctl(handle.as_raw_fd(), 0x8008_4803, info.as_mut_ptr()) };
        if status < 0 || u32::from_ne_bytes(info[..4].try_into().ok()?) != 3
            || u16::from_ne_bytes(info[4..6].try_into().ok()?) != 0x264a
            || u16::from_ne_bytes(info[6..8].try_into().ok()?) != pid
        {
            return None;
        }
        (unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0)
            .then_some(handle)
    })
}

pub(crate) fn transfer(mut handle: &Handle, buffer: &mut [u8], write: bool, timeout: u32) -> Option<usize> {
    let mut fd = libc::pollfd {
        fd: handle.as_raw_fd(),
        events: if write { libc::POLLOUT } else { libc::POLLIN },
        revents: 0,
    };
    if unsafe { libc::poll(&mut fd, 1, timeout as i32) } <= 0 || fd.revents & fd.events == 0 {
        return None;
    }
    if write { handle.write(buffer).ok() } else { handle.read(buffer).ok() }
}

pub(crate) fn feature(handle: &Handle, report: &mut [u8; 8], write: bool) -> bool {
    // HIDIOCSFEATURE(8) and HIDIOCGFEATURE(8), from linux/hidraw.h.
    let request = if write { 0xc008_4806 } else { 0xc008_4807 };
    unsafe { libc::ioctl(handle.as_raw_fd(), request, report.as_mut_ptr()) == 8 }
}

pub(crate) fn has_other_owner(handle: &Handle) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(target) = handle.metadata() else { return true; };
    let Ok(processes) = fs::read_dir("/proc") else { return true; };
    let ours = std::process::id().to_string();
    processes.flatten().any(|process| {
        let id = process.file_name();
        let Some(id) = id.to_str() else { return false; };
        if id == ours || !id.bytes().all(|c| c.is_ascii_digit()) { return false; }
        fs::read_dir(process.path().join("fd")).is_ok_and(|mut fds| {
            fds.any(|fd| fd.ok().and_then(|fd| fs::metadata(fd.path()).ok())
                .is_some_and(|meta| meta.file_type().is_char_device() && meta.rdev() == target.rdev()))
        })
    })
}
