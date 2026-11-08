use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aur-prefetch"))
}

#[test]
fn help_lists_every_subcommand() {
    let output = binary().arg("--help").output().expect("run --help");

    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for name in ["init", "check", "fetch", "install", "clean", "doctor"] {
        assert!(text.contains(name), "help is missing {name}");
    }
}

// Only commands that are still stubs may run here. Executing an implemented
// command from `cargo test` would touch the development host.
#[test]
fn every_stub_subcommand_reports_that_it_is_not_implemented() {
    let name = "doctor";
    let output = binary().arg(name).output().expect("run subcommand");

    assert!(output.status.success(), "{name} did not exit 0");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("{name}: not implemented yet")
    );
}
