use linefeed::ReadResult;
use std::collections::HashMap;
use std::env::set_current_dir;
use std::fs::File;
use std::io::{stdout, BufRead, BufReader, ErrorKind, Write};
use std::os::windows::process::ExitStatusExt;
use std::path::{Component, PathBuf};
use std::process::{Command, ExitStatus, Output};

fn tokenize(input: &str) -> Result<Vec<String>, ErrorKind> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quotes = false;

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            _ => current.push(c),
        }
    }

    if in_quotes {
        return Err(ErrorKind::InvalidInput); // niedomknięty cudzysłów
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

struct Shell {
    home_dir: PathBuf,
    current_dir: PathBuf,
    variables: HashMap<String, String>,
    aliases: HashMap<String, String>,
    exit_status: ExitStatus,
}

impl Shell {
    pub fn new() -> Self {
        let mut state = Self::default();

        state.home_dir = dirs::home_dir().expect("Failed to get home directory");
        state.current_dir = state.home_dir.clone();
        state.variables = std::env::vars().collect::<HashMap<String, String>>();
        state.variables.insert(
            "PWD".to_string(),
            state.current_dir.clone().to_string_lossy().parse().unwrap(),
        );
        state.variables.insert(
            "HOME".to_string(),
            state.home_dir.clone().to_string_lossy().parse().unwrap(),
        );
        set_current_dir(state.current_dir.clone()).unwrap();
        let t = get_coreutils_commands().expect("Failed to get coreutils commands");
        for command in t.iter() {
            state
                .aliases
                .insert(command.to_string(), format!("coreutils {}", command));
        }

        state
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            home_dir: PathBuf::new(),
            current_dir: PathBuf::new(),
            variables: HashMap::new(),
            aliases: HashMap::new(),
            exit_status: ExitStatus::default(),
        }
    }
}

struct CommandContainer {
    program: String,
    args: Vec<String>,
}

impl CommandContainer {
    fn new(program: String, args: Vec<String>) -> Self {
        Self { program, args }
    }
}

impl Shell {
    pub fn execute(&mut self, buffer: &str) -> Result<(), ErrorKind> {
        let mut command = self.parse_command(buffer)?;
        self.execute_command(&mut command)?;
        Ok(())
    }

    fn parse_command(&self, buffer: &str) -> Result<CommandContainer, ErrorKind> {
        if buffer.is_empty() {
            return Err(ErrorKind::InvalidInput);
        }

        let buffer = buffer.trim();

        let mut tokens = tokenize(buffer)?;
        tokens = self.resolve_alias(&tokens[0]);
        tokens.extend(
            tokenize(buffer)?[1..]
                .iter()
                .map(|a| self.resolve_variable(a)),
        );

        let command = CommandContainer::new(tokens[0].clone(), tokens[1..].to_vec());

        // println!("LOG: {} {}", &command.program, &command.args.join(" "));

        Ok(command)
    }

    fn execute_command(&mut self, command: &mut CommandContainer) -> Result<(), ErrorKind> {
        match command.program.as_str() {
            "clear" => clear_terminal(),
            "cd" => self.change_directory(&command.args),
            "export" => {
                for arg in &command.args {
                    self.add_variable(&arg);
                }
                Ok(())
            }
            "alias" => {
                for arg in &command.args {
                    self.add_alias(&arg);
                }
                Ok(())
            }
            "exit" => Err(ErrorKind::Interrupted),
            _ => self.execute_external_command(command),
        }
    }

    fn execute_external_command(
        &mut self,
        command: &mut CommandContainer,
    ) -> Result<(), ErrorKind> {
        match Command::new(command.program.clone())
            .args(command.args.clone())
            .envs(self.variables.clone())
            .status()
        {
            Ok(status) => {
                self.exit_status = status;
                Ok(())
            }
            Err(err) => Err(err.kind()),
        }
    }

    fn get_result_of_external_command(
        &mut self,
        command: &mut CommandContainer,
    ) -> Result<Output, ErrorKind> {
        match Command::new(command.program.clone())
            .args(command.args.clone())
            .envs(self.variables.clone())
            .output()
        {
            Ok(output) => Ok(output),
            Err(err) => Err(err.kind()),
        }
    }

    fn resolve_alias(&self, command: &str) -> Vec<String> {
        let mut tokens = vec![command.to_owned()];
        let mut seen = std::collections::HashSet::new();

        while let Some(alias) = self.aliases.get(&tokens[0]) {
            if !seen.insert(tokens[0].clone()) || seen.len() > 32 {
                break;
            }

            let mut alias_tokens = match tokenize(alias) {
                Ok(tokens) => tokens,
                Err(_) => continue,
            };

            alias_tokens.extend(tokens.drain(1..));
            tokens = alias_tokens;
        }

        tokens
    }

    fn resolve_variable(&self, arg: &str) -> String {
        if let Some(name) = arg.strip_prefix('$') {
            if name == "?" {
                return self.exit_status.code().unwrap_or(0).to_string();
            }

            self.variables
                .get(name)
                .cloned()
                .unwrap_or_else(|| arg.to_owned())
        } else {
            arg.to_owned()
        }
    }

    pub fn change_directory(&mut self, args: &[String]) -> Result<(), ErrorKind> {
        if args.len() > 1 {
            self.exit_status = ExitStatus::from_raw(1);
            return Err(ErrorKind::InvalidInput);
        }

        let new_dir = match args.get(0) {
            Some(path) => {
                let path = if path.starts_with('~') {
                    let rest = &path[1..];
                    self.home_dir.join(rest)
                } else {
                    self.current_dir.join(path)
                };

                path
            }
            None => self.home_dir.clone(),
        };

        let new_dir = normalize_path(new_dir);

        if set_current_dir(new_dir.clone()).is_err() {
            return Err(ErrorKind::InvalidInput);
        }

        if new_dir.is_dir() {
            self.current_dir = new_dir.clone();
            self.variables.insert("PWD".to_string(), new_dir.to_string_lossy().to_string());
            self.exit_status = ExitStatus::from_raw(0);
            Ok(())
        } else {
            self.exit_status = ExitStatus::from_raw(1);
            Err(ErrorKind::InvalidInput)
        }
    }

    fn add_variable(&mut self, text: &str) {
        if let Some((key, val)) = text.split_once('=') {
            let val = val.trim_matches('"');
            self.variables
                .insert(key.trim().to_string(), val.to_string());
        }
    }

    fn add_alias(&mut self, text: &str) {
        if let Some((key, val)) = text.split_once('=') {
            let val = val.trim_matches('"');
            self.aliases.insert(key.trim().to_string(), val.to_string());
        }
    }

    fn get_prompt(&mut self) -> String {
        let prompt_command = self
            .variables
            .get("PROMPT")
            .cloned()
            .unwrap_or_else(|| "echo $PWD >".to_string());

        let mut parsed_prompt = match self.parse_command(&prompt_command) {
            Ok(command) => command,
            Err(_) => return "".to_string(),
        };

        match self.get_result_of_external_command(&mut parsed_prompt) {
            Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
            Err(_) => "".to_string(),
        }
    }
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

fn clear_terminal() -> Result<(), ErrorKind> {
    print!("\x1B[2J\x1B[1;1H");
    stdout().flush().unwrap();

    Ok(())
}

fn load_rc(state: &mut Shell) {
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

        state.execute(&l).unwrap();
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
    let mut shell = Shell::new();
    load_rc(&mut shell);

    let interface = linefeed::Interface::new("wpcsh").expect("Failed to initialize interface");

    let history_path = shell.home_dir.join(".wpcsh_history");
    let _ = interface.load_history(&history_path);

    loop {

        let prompt = shell.get_prompt();

        print!("{}", prompt);
        stdout().flush().unwrap();

        let readline = interface.read_line();

        match readline {
            Ok(read_result) => match read_result {
                ReadResult::Input(line) => {
                    interface.add_history(line.clone());
                    if line.is_empty() {
                        continue;
                    }
                    let _ = shell.execute(&line);
                }
                ReadResult::Signal(_) => {}
                ReadResult::Eof => {}
            },
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
        interface
            .save_history(&history_path)
            .expect("Failed to save history");

        println!();
    }
}
