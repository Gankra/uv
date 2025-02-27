#![cfg(feature = "python")]

use std::ffi::OsString;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use fs_err::PathExt;
use uv_fs::Simplified;

use crate::common::{
    create_bin_with_executables, get_bin, uv_snapshot, TestContext, EXCLUDE_NEWER,
};

mod common;

struct VenvTestContext {
    cache_dir: assert_fs::TempDir,
    temp_dir: assert_fs::TempDir,
    venv: ChildPath,
    bin: OsString,
}

impl VenvTestContext {
    fn new(python_versions: &[&str]) -> Self {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let bin = create_bin_with_executables(&temp_dir, python_versions)
            .expect("Failed to create bin dir");
        let venv = temp_dir.child(".venv");
        Self {
            cache_dir: assert_fs::TempDir::new().unwrap(),
            temp_dir,
            venv,
            bin,
        }
    }

    fn venv_command(&self) -> Command {
        let mut command = Command::new(get_bin());
        command
            .arg("venv")
            .arg("--cache-dir")
            .arg(self.cache_dir.path())
            .arg("--exclude-newer")
            .arg(EXCLUDE_NEWER)
            .env("UV_TEST_PYTHON_PATH", self.bin.clone())
            .current_dir(self.temp_dir.path());
        command
    }

    fn filters(&self) -> Vec<(String, String)> {
        // On windows, a directory can have multiple names (https://superuser.com/a/1666770), e.g.
        // `C:\Users\KONSTA~1` and `C:\Users\Konstantin` are the same.
        let venv_full = regex::escape(&self.venv.display().to_string());
        let mut filters = vec![(venv_full, ".venv".to_string())];

        // For mac, otherwise it shows some /var/folders/ path.
        if let Ok(canonicalized) = self.venv.path().fs_err_canonicalize() {
            let venv_full = regex::escape(&canonicalized.simplified_display().to_string());
            filters.push((venv_full, ".venv".to_string()));
        }

        filters.push((
            r"interpreter at: .+".to_string(),
            "interpreter at: [PATH]".to_string(),
        ));
        filters.push((
            r"Activate with: (?:.*)\\Scripts\\activate".to_string(),
            "Activate with: source .venv/bin/activate".to_string(),
        ));
        filters
    }
}

#[test]
fn create_venv() {
    let context = VenvTestContext::new(&["3.12"]);

    // Create a virtual environment at `.venv`.
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());

    // Create a virtual environment at the same location, which should replace it.
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_defaults_to_cwd() {
    let context = VenvTestContext::new(&["3.12"]);
    uv_snapshot!(context.filters(), context.venv_command()
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn seed() {
    let context = VenvTestContext::new(&["3.12"]);
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--seed")
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
     + pip==24.0
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn seed_older_python_version() {
    let context = VenvTestContext::new(&["3.10"]);
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--seed")
        .arg("--python")
        .arg("3.10")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.10.13 interpreter at: [PATH]
    Creating virtualenv at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.43.0
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_unknown_python_minor() {
    let context = VenvTestContext::new(&["3.12"]);

    let mut command = context.venv_command();
    command
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.15")
        .env("UV_NO_WRAP", "1");
    if cfg!(windows) {
        uv_snapshot!(&mut command, @r###"
        success: false
        exit_code: 1
        ----- stdout -----

        ----- stderr -----
          × No Python 3.15 found through `py --list-paths` or in `PATH`. Is Python 3.15 installed?
        "###
        );
    } else {
        uv_snapshot!(&mut command, @r###"
        success: false
        exit_code: 1
        ----- stdout -----

        ----- stderr -----
          × No Python 3.15 in `PATH`. Is Python 3.15 installed?
        "###
        );
    }

    context.venv.assert(predicates::path::missing());
}

#[test]
fn create_venv_unknown_python_patch() {
    let context = VenvTestContext::new(&["3.12"]);

    let filters = &[
        (
            r"Using Python 3\.\d+\.\d+ interpreter at: .+",
            "Using Python [VERSION] interpreter at: [PATH]",
        ),
        (
            r"No Python 3\.8\.0 found through `py --list-paths` or in `PATH`\. Is Python 3\.8\.0 installed\?",
            "No Python 3.8.0 in `PATH`. Is Python 3.8.0 installed?",
        ),
    ];
    uv_snapshot!(filters, context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.8.0")
        .env("UV_NO_WRAP", "1"), @r###"
    success: false
    exit_code: 1
    ----- stdout -----

    ----- stderr -----
      × No Python 3.8.0 in `PATH`. Is Python 3.8.0 installed?
    "###
    );

    context.venv.assert(predicates::path::missing());
}

#[test]
fn create_venv_python_patch() {
    let context = VenvTestContext::new(&["3.12.1"]);

    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12.1")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn file_exists() -> Result<()> {
    let context = VenvTestContext::new(&["3.12"]);

    // Create a file at `.venv`. Creating a virtualenv at the same path should fail.
    context.venv.touch()?;

    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: false
    exit_code: 1
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    uv::venv::creation

      × Failed to create virtualenv
      ╰─▶ File exists at `.venv`
    "###
    );

    Ok(())
}

#[test]
fn empty_dir_exists() -> Result<()> {
    let context = VenvTestContext::new(&["3.12"]);

    // Create an empty directory at `.venv`. Creating a virtualenv at the same path should succeed.
    context.venv.create_dir_all()?;
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn non_empty_dir_exists() -> Result<()> {
    let context = VenvTestContext::new(&["3.12"]);

    // Create a non-empty directory at `.venv`. Creating a virtualenv at the same path should fail.
    context.venv.create_dir_all()?;
    context.venv.child("file").touch()?;

    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env("UV_NO_WRAP", "1"), @r###"
    success: false
    exit_code: 1
    ----- stdout -----

    ----- stderr -----
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    uv::venv::creation

      × Failed to create virtualenv
      ╰─▶ The directory `.venv` exists, but it's not a virtualenv
    "###
    );

    Ok(())
}

#[test]
#[cfg(windows)]
fn windows_shims() -> Result<()> {
    let context = VenvTestContext::new(&["3.9", "3.8"]);
    let shim_path = context.temp_dir.child("shim");

    let py38 = std::env::split_paths(&context.bin)
        .last()
        .expect("create_bin_with_executables to set up the python versions");
    // We want 3.8 and the first version should be 3.9.
    // Picking the last is necessary to prove that shims work because the python version selects
    // the python version from the first path segment by default, so we take the last to prove it's not
    // returning that version.
    assert!(py38.to_str().unwrap().contains("3.8"));

    // Write the shim script that forwards the arguments to the python3.8 installation.
    fs_err::create_dir(&shim_path)?;
    fs_err::write(
        shim_path.child("python.bat"),
        format!("@echo off\r\n{}/python.exe %*", py38.display()),
    )?;

    // Create a virtual environment at `.venv`, passing the redundant `--clear` flag.
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .env("UV_TEST_PYTHON_PATH", format!("{};{}", shim_path.display(), context.bin.simplified_display())), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    warning: virtualenv's `--clear` has no effect (uv always clears the virtual environment).
    Using Python 3.8.12 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn virtualenv_compatibility() {
    let context = VenvTestContext::new(&["3.12"]);

    // Create a virtual environment at `.venv`, passing the redundant `--clear` flag.
    uv_snapshot!(context.filters(), context.venv_command()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    warning: virtualenv's `--clear` has no effect (uv always clears the virtual environment).
    Using Python 3.12.1 interpreter at: [PATH]
    Creating virtualenv at: .venv
    Activate with: source .venv/bin/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn verify_pyvenv_cfg() {
    let context = TestContext::new("3.12");
    let venv = context.temp_dir.child(".venv");
    let pyvenv_cfg = venv.child("pyvenv.cfg");

    venv.assert(predicates::path::is_dir());

    // Check pyvenv.cfg exists
    pyvenv_cfg.assert(predicates::path::is_file());

    // Check if "uv = version" is present in the file
    let version = env!("CARGO_PKG_VERSION").to_string();
    let search_string = format!("uv = {version}");
    pyvenv_cfg.assert(predicates::str::contains(search_string));
}

/// Ensure that a nested virtual environment uses the same `home` directory as the parent.
#[test]
fn verify_nested_pyvenv_cfg() -> Result<()> {
    let context = VenvTestContext::new(&["3.12"]);

    // Create a virtual environment at `.venv`.
    context
        .venv_command()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");

    // Check pyvenv.cfg exists
    pyvenv_cfg.assert(predicates::path::is_file());

    // Extract the "home" line from the pyvenv.cfg file.
    let contents = fs_err::read_to_string(pyvenv_cfg.path())?;
    let venv_home = contents
        .lines()
        .find(|line| line.starts_with("home"))
        .expect("home line not found");

    // Now, create a virtual environment from within the virtual environment.
    let subvenv = context.temp_dir.child(".subvenv");
    context
        .venv_command()
        .arg(subvenv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env("VIRTUAL_ENV", context.venv.as_os_str())
        .env("UV_NO_WRAP", "1")
        .assert()
        .success();

    let sub_pyvenv_cfg = subvenv.child("pyvenv.cfg");

    // Extract the "home" line from the pyvenv.cfg file.
    let contents = fs_err::read_to_string(sub_pyvenv_cfg.path())?;
    let sub_venv_home = contents
        .lines()
        .find(|line| line.starts_with("home"))
        .expect("home line not found");

    // Check that both directories point to the same home.
    assert_eq!(sub_venv_home, venv_home);

    Ok(())
}
