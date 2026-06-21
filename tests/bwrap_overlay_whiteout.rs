use std::ffi::CString;
use std::fs::{self, File};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::process::Command;

use temp_dir::TempDir;

#[test]
// Verifies that bwrap's overlay mount interprets user.overlay.whiteout xattrs.
// Requires bwrap, /bin/sh and working unprivileged overlay support.
fn bwrap_overlay_honors_user_xattr_whiteout() {
    let temp_dir = TempDir::new().unwrap();
    let lower = temp_dir.path().join("lower");
    let upper = temp_dir.path().join("upper");
    fs::create_dir_all(&lower).unwrap();
    fs::create_dir_all(&upper).unwrap();
    fs::write(lower.join("deleted"), b"lower content").unwrap();
    File::create(upper.join("deleted")).unwrap();
    set_xattr(&upper.join("deleted"), "user.overlay.whiteout", b"y")
        .expect("failed to set whiteout xattr");

    let output = Command::new("bwrap")
        .arg("--clearenv")
        .arg("--unshare-all")
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--dir")
        .arg("/tmp/merged")
        .arg("--overlay-src")
        .arg(&lower)
        .arg("--overlay-src")
        .arg(&upper)
        .arg("--ro-overlay")
        .arg("/tmp/merged")
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg("test ! -e /tmp/merged/deleted")
        .output()
        .expect("failed to spawn bwrap");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("bwrap overlay did not hide whiteout target: {stderr}");
    }
}

fn set_xattr(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let name = CString::new(name).unwrap();
    let result = unsafe {
        libc::setxattr(
            path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
