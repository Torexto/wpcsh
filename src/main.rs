mod posix_commands;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write, stdin, stdout};
use std::path::{Component, PathBuf};
use std::process::{Command, ExitStatus};

struct State {
    home_dir: PathBuf,
    current_dir: PathBuf,
    coreutils_commands: Vec<String>,
    variables: HashMap<String, String>,
    aliases: HashMap<String, String>,
    exit_status: ExitStatus,
}

impl State {
    pub fn new() -> Self {
        let mut state = Self::default();

        state.home_dir = dirs::home_dir().expect("Failed to get home directory");
        state.current_dir = state.home_dir.clone();
        state.variables = std::env::vars().collect::<HashMap<String, String>>();
        state.coreutils_commands =
            get_coreutils_commands().expect("Failed to get coreutils commands");

        state
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            home_dir: PathBuf::new(),
            current_dir: PathBuf::new(),
            coreutils_commands: Vec::new(),
            variables: HashMap::new(),
            aliases: HashMap::new(),
            exit_status: ExitStatus::default(),
        }
    }
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

        execute_command(state, l)
            .unwrap()
            .expect("TODO: panic message");
    }
}

fn resolve_alias(state: &State, command: &str, args: &[String]) -> (String, Vec<String>) {
    if let Some(alias_cmd) = state.aliases.get(command) {
        let mut parts = alias_cmd.split_whitespace();
        if let Some(first) = parts.next() {
            let new_args = parts
                .map(|s| s.to_string())
                .chain(args.iter().map(|s| s.to_string()))
                .collect();
            return (first.to_string(), new_args);
        }
    }
    (
        command.to_string(),
        args.iter().map(|s| s.to_string()).collect(),
    )
}

fn add_variable(state: &mut State, text: &str) {
    if let Some((key, val)) = text.split_once('=') {
        let val = val.trim_matches('"');
        state
            .variables
            .insert(key.trim().to_string(), val.to_string());
    }
}

fn add_alias(state: &mut State, text: &str) {
    if let Some((key, val)) = text.split_once('=') {
        let val = val.trim_matches('"');
        state
            .aliases
            .insert(key.trim().to_string(), val.to_string());
    }
}

fn get_var(state: &State, var_name: &str) -> Option<String> {
    if var_name == "?" {
        Some(state.exit_status.code().unwrap_or(0).to_string())
    } else {
        None
    }
}

fn execute_command(state: &mut State, buffer: String) -> Option<Result<(), String>> {
    let elements: Vec<String> = buffer.trim().split_whitespace().map(String::from).collect();

    let command = elements.get(0).unwrap_or(&String::new()).to_string();
    let command = state.aliases.get(&command).unwrap_or(&command).to_owned();

    let args: Vec<String> = elements[1..].iter().map(String::from).collect();

    let mut args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            if arg.starts_with("$") {
                let key = &arg[1..];
                state
                    .variables
                    .get(key)
                    .unwrap_or(&String::new())
                    .to_string()
            } else {
                arg
            }
        })
        .collect();

    if state
        .coreutils_commands
        .binary_search(&command.to_string())
        .is_ok()
    {
        let mut new_args: Vec<String> = vec![command];
        new_args.append(&mut args);
        return exec_external(state, "coreutils", &new_args);
    };

    match command.as_str() {
        "clear" => Some(clear_terminal()),
        "cd" => Some(posix_commands::cd::cd(state, args)),
        "export" => {
            for arg in args {
                add_variable(state, arg.as_str());
            }
            Some(Ok(()))
        }
        "alias" => {
            for arg in args {
                add_alias(state, arg.as_str());
            }
            Some(Ok(()))
        }
        "exit" => None,
        _ => {
            let (exec_command, exec_args) = resolve_alias(state, &command, args.as_slice());
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

fn exec_external(
    state: &mut State,
    command: &str,
    args: &Vec<String>,
) -> Option<Result<(), String>> {
    let (cmd, exec_args) = resolve_alias(state, command, args);

    match Command::new(&cmd).args(&exec_args).envs(state.variables.clone()).status() {
        Ok(status) => {
            state.exit_status = status;
            Some(Ok(()))
        }
        Err(_) => Some(Err(format!("wpcsh: {}: command not found", cmd))),
    }
}

fn get_coreutils_commands() -> std::io::Result<Vec<String>> {
    let output = Command::new("coreutils").arg("--list").output()?;

    let coreutils_commands = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let coreutils_commands = coreutils_commands
        .split_whitespace()
        .map(String::from)
        .collect::<Vec<String>>();

    Ok(coreutils_commands)
}

fn main() {
    let mut state = State::new();
    load_rc(&mut state);

    let mut buf_reader = BufReader::new(stdin());
    let mut buff = String::new();

    loop {
        std::env::set_current_dir(&state.current_dir).unwrap();
        buff.clear();
        print_prefix(&state);

        match buf_reader.read_line(&mut buff) {
            Ok(bytes) => {
                if bytes == 0 {
                    continue;
                }
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
