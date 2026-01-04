use wpcsh::Shell;

#[cfg(unix)]
fn install_signal_handlers() {
    use signal_hook::consts::{SIGHUP, SIGTERM};
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new([SIGTERM, SIGHUP]).expect("signals");

    std::thread::spawn(move || {
        for _ in signals.forever() {
            std::process::exit(0);
        }
    });
}

#[cfg(unix)]
fn is_interactive() -> bool {
    atty::is(atty::Stream::Stdin)
}

#[cfg(unix)]
fn is_login_shell() -> bool {
    std::env::args()
        .next()
        .map(|a| a.starts_with('-'))
        .unwrap_or(false)
}

fn main() {
    #[cfg(unix)]
    {
        install_signal_handlers();

        let mut shell = Shell::new();

        let login = is_login_shell();
        let interactive = is_interactive();

        if login {
            shell.load_login_config();
        }

        if interactive {
            shell.run_interactive();
        } else {
            shell.run_non_interactive();
        }
    }

    #[cfg(windows)]
    {
        let mut shell = Shell::new();
        shell.run_interactive();
    }
}
