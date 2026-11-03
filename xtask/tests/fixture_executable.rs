//! Stable installation of executable integration-test fixtures.

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, io};

pub(crate) fn install_executable(path: &Path, contents: &str) {
    install_executable_bytes(path, contents.as_bytes()).expect("install executable fixture");
}

pub(crate) fn install_executable_copy(source: &Path, destination: &Path) {
    let contents = fs::read(source).expect("read executable fixture source");
    install_executable_bytes(destination, &contents).expect("install copied executable fixture");
}

fn install_executable_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    let file_name = path.file_name().expect("executable fixture file name");
    let staging = path.with_file_name(format!(".{}.installing", file_name.to_string_lossy()));
    if staging.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("staging path already exists: {}", staging.display()),
        ));
    }
    let mut writer = Command::new("/usr/bin/tee")
        .arg("--")
        .arg(&staging)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let write_result = writer
        .stdin
        .take()
        .expect("fixture writer stdin")
        .write_all(contents);
    let output = writer.wait_with_output()?;
    write_result?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "fixture writer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o555))?;
    fs::rename(staging, path)
}

#[test]
fn executable_fixtures_are_complete_read_only_and_atomically_replaceable() {
    let directory = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("executable fixture directory");
    let executable = directory.path().join("fixture.sh");

    install_executable(&executable, "#!/bin/sh\nprintf first\n");
    assert_eq!(
        fs::read_to_string(&executable).expect("first fixture"),
        "#!/bin/sh\nprintf first\n"
    );
    assert_eq!(
        fs::metadata(&executable)
            .expect("first fixture metadata")
            .permissions()
            .mode()
            & 0o777,
        0o555,
        "a published fixture must not admit another writer"
    );

    install_executable(&executable, "#!/bin/sh\nprintf second\n");
    assert_eq!(
        fs::read_to_string(&executable).expect("replacement fixture"),
        "#!/bin/sh\nprintf second\n"
    );
    let output = std::process::Command::new(&executable)
        .output()
        .expect("execute replacement fixture");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"second");
}
