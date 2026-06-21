use std::fs::{File, OpenOptions};
use std::io::{self, Read as _};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use temp_dir::TempDir;

#[test]
// Verifies that bwrap --lock-file takes a POSIX fcntl read lock that the host
// observes as conflicting. Requires bwrap, /bin/sleep and working unprivileged namespaces.
fn bwrap_lock_file_locks() {
    let temp_dir = TempDir::new().unwrap();
    let lock_path = temp_dir.path().join("lock");
    File::create(&lock_path).unwrap();

    let mut child = spawn_bwrap_with_lock_file(&lock_path).expect("failed to spawn bwrap");

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    let locked = wait_until_locked(&mut child, &lock_file, Duration::from_secs(2));

    child.kill().ok();
    child.wait().ok();

    assert!(locked, "host did not observe bwrap's fcntl lock");
}

fn spawn_bwrap_with_lock_file(lock_path: &Path) -> io::Result<Child> {
    Command::new("bwrap")
        .arg("--clearenv")
        .arg("--unshare-all")
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--ro-bind")
        .arg(lock_path)
        .arg("/tmp/bwrap-lock-file-test-lock")
        .arg("--lock-file")
        .arg("/tmp/bwrap-lock-file-test-lock")
        .arg("--")
        .arg("/bin/sleep")
        .arg("10")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

fn wait_until_locked(child: &mut Child, file: &File, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if write_lock_would_block(file).unwrap() {
            return true;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                pipe.read_to_string(&mut stderr).unwrap();
            }
            panic!("bwrap exited before taking the lock: {status}; stderr: {stderr}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn write_lock_would_block(file: &File) -> io::Result<bool> {
    let lock = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };

    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) };
    if result == 0 {
        unlock(file)?;
        return Ok(false);
    }

    let error = io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EAGAIN)) {
        return Ok(true);
    }
    Err(error)
}

fn unlock(file: &File) -> io::Result<()> {
    let lock = libc::flock {
        l_type: libc::F_UNLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };

    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
