mod posix_commands;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write, stdin, stdout};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};

struct State {
    home_dir: PathBuf,
    current_dir: PathBuf,
    vars: HashMap<String, String>,
    aliases: HashMap<String, String>,
    exit_status: ExitStatus,
}

impl Default for State {
    fn default() -> Self {
        let home_dir = dirs::home_dir().expect("Failed to get home directory");
        Self {
            home_dir: home_dir.clone(),
            current_dir: home_dir,
            vars: HashMap::new(),
            aliases: HashMap::new(),
            exit_status: ExitStatus::default(),
        }
    }
}

fn path_to_str(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}

fn print_prefix(state: &State) {
    let status_code = state.exit_status.code().unwrap_or(0).to_string();

    let starship_prompt = Command::new("starship")
        .arg("prompt")
        .env("STARSHIP_SHELL", "wpcsh") // własny shell
        .env("PWD", state.current_dir.to_str().unwrap_or(""))
        .env("STARSHIP_CMD_STATUS", &status_code)
        .env("HOME", &state.home_dir)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|| format!("{}>", state.current_dir.to_str().unwrap_or(""))); // fallback

    print!("{starship_prompt}");
    stdout().flush().unwrap();
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other),
        }
    }

    result
}

fn clear_terminal() -> Result<(), String> {
    print!("\x1B[2J\x1B[1;1H");
    stdout().flush().unwrap();

    Ok(())
}

fn load_rc(state: &mut State) {
    let rc_path = state.home_dir.join(".wpcshrc");
    if !rc_path.exists() {
        return;
    }

    let file = match File::open(&rc_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let reader = BufReader::new(file);

    for line in reader.lines().flatten() {
        let l = line.trim().to_string();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }

        if l.starts_with("export ") {
            handle_export(state, &l[7..]);
        } else if l.starts_with("alias ") {
            handle_alias(state, &l[6..]);
        }
    }
}

fn resolve_alias(state: &State, command: &str, args: &[&str]) -> (String, Vec<String>) {
    if let Some(alias_cmd) = state.aliases.get(command) {
        let mut parts = alias_cmd.split_whitespace();
        if let Some(first) = parts.next() {
            let new_args = parts.map(|s| s.to_string()).chain(args.iter().map(|s| s.to_string())).collect();
            return (first.to_string(), new_args);
        }
    }
    (command.to_string(), args.iter().map(|s| s.to_string()).collect())
}


fn handle_export(state: &mut State, text: &str) {
    if let Some((key, val)) = text.split_once('=') {
        let val = val.trim_matches('"');
        state.vars.insert(key.trim().to_string(), val.to_string());
        unsafe { std::env::set_var(key.trim(), val) };
    }
}

fn handle_alias(state: &mut State, text: &str) {
    if let Some((name, cmd)) = text.split_once('=') {
        let cmd = cmd.trim_matches('"');
        state.aliases.insert(name.trim().to_string(), cmd.to_string());
    }
}

fn execute_command(state: &mut State, buffer: String) -> Option<Result<(), String>> {
    let elements: Vec<&str> = buffer.trim().split_whitespace().collect();
    let command = match elements.get(0) {
        Some(c) => *c,
        None => return Some(Ok(())),
    };
    let args = &elements[1..];

    match command {
        "clear" => Some(clear_terminal()),
        "cd" => Some(posix_commands::cd::cd(state, args)),
        "ls" => Some(posix_commands::ls::ls(state, args)),
        "pwd" => Some(posix_commands::pwd::pwd(state)),
        "echo" => Some(posix_commands::echo::echo(Some(state), args)),
        "export" => {
            for arg in args {
                handle_export(state, arg);
            }
            Some(Ok(()))
        }
        "alias" => {
            for arg in args {
                handle_alias(state, arg);
            }
            Some(Ok(()))
        }
        "exit" => None,
        _ => {
            let (exec_command, exec_args) = resolve_alias(state, command, args);
            let mut cmd = Command::new(exec_command);
            cmd.args(exec_args);
            match cmd.status() {
                Ok(status) => {
                    state.exit_status = status;
                    Some(Ok(()))
                }
                Err(_) => Some(Err(format!("wpcsh: {}: command not found", command))),
            }
        }
    }
}

fn main() {
    let mut state = State::default();
    load_rc(&mut state);

    let mut buf_reader = BufReader::new(stdin());
    let mut buff = String::new();

    loop {
        std::env::set_current_dir(&state.current_dir).unwrap();
        buff.clear();
        print_prefix(&state);

        match buf_reader.read_line(&mut buff) {
            Ok(_) => {
                match execute_command(&mut state, buff.clone()) {
                    Some(result) => {
                        if let Err(err) = result {
                            eprintln!("{}", err);
                        }
                    }
                    None => break,
                }
            }
            Err(_) => break,
        }
    }
}
