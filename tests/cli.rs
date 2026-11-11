use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aur-prefetch"))
}

#[test]
fn help_lists_every_subcommand() {
    let output = binary().arg("--help").output().expect("run --help");

    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for name in [
        "init", "check", "fetch", "install", "clean", "doctor", "tui",
    ] {
        assert!(text.contains(name), "help is missing {name}");
    }
}

#[test]
fn tui_help_lists_the_safe_flags() {
    let output = binary()
        .args(["tui", "--help"])
        .output()
        .expect("run tui --help");

    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("--render-once"));
    assert!(text.contains("--ascii"));
}
