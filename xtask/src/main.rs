//! `cargo xtask <command>`: the project's automation that needs more than a script.
//!
//! * `register [--no-build]`: install this checkout's release build for the current
//!   user on Linux: the viewer and `review3d-cli` on the PATH, a menu entry, and the
//!   default application for FBX, glb and glTF files (`docs/ARCHITECTURE.md`, D32).
//! * `unregister`: take all of it out again.

use std::process::ExitCode;

mod register;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        usage();
        return ExitCode::from(2);
    };
    let result = match command.as_str() {
        "register" => register::register(rest),
        "unregister" => register::unregister(rest),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <command>

  register [--no-build]   install this checkout's release build for the current user
                          (Linux): ~/.local/lib/3d-review, with 3d-review and review3d-cli
                          linked into ~/.local/bin, a menu entry, and the default
                          application for .fbx, .glb and .gltf files. Builds first unless
                          --no-build; run it again after a rebuild to update the install
  unregister              take all of that out again (settings and logs stay)"
    );
}
