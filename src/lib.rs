mod token;

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

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

#[derive(Debug, Default)]
pub struct Shell {
    home_dir: PathBuf,
    current_dir: PathBuf,
    variables: HashMap<String, String>,
    aliases: HashMap<String, String>,
    exit_status: ExitStatus,
}

impl Shell {
    pub fn new() -> Result<Self, ErrorKind> {
        let home_dir = dirs::home_dir().ok_or(ErrorKind::NotFound)?;

        use std::env;

        let mut shell = Self {
            home_dir: home_dir.clone(),
            current_dir: home_dir,
            variables: env::vars().collect::<HashMap<String, String>>(),
            aliases: HashMap::new(),
            exit_status: ExitStatus::default(),
        };

        shell.set_default_variables();

        if env::set_current_dir(shell.current_dir.clone()).is_err() {
            return Err(ErrorKind::InvalidInput);
        };

        shell.set_coreutils_alias();

        Ok(shell)
    }

    fn set_default_variables(&mut self) {
        self.variables.insert(
            "PWD".to_string(),
            self.current_dir.to_string_lossy().to_string(),
        );
        self.variables.insert(
            "HOME".to_string(),
            self.home_dir.to_string_lossy().to_string(),
        );
        self.variables.insert(
            "SHELL".to_string(),
            match std::env::current_exe() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => "".to_string(),
            },
        );
    }

    fn set_coreutils_alias(&mut self) {
        #[cfg(windows)]
        {
            let commands = get_coreutils_commands().expect("Failed to get coreutils commands");

            for command in commands.iter() {
                self.aliases
                    .insert(command.to_string(), format!("coreutils {}", command));
            }
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

        // use crate::token;
        // let mut lexer = token::Lexer::new(buffer);
        //
        // loop {
        //     let token = lexer.next_token();
        //     println!("{:?}", token);
        //     if token == token::Token::Eof {
        //         break;
        //     }
        // }

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
            "clear" => self.clear_terminal(),
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
            "exit" => self.exit(command),
            "source" => self.source_command(command),
            _ => self.execute_external_command(command),
        }
    }

    fn exit(&mut self, command: &CommandContainer) -> Result<(), ErrorKind> {
        let code = command
            .args
            .get(0)
            .and_then(|a| a.parse::<i32>().ok())
            .unwrap_or(0);

        std::process::exit(code);
    }

    fn source_command(&mut self, command: &mut CommandContainer) -> Result<(), ErrorKind> {
        let path = match command.args.get(0) {
            Some(path) => PathBuf::from(path),
            None => return Err(ErrorKind::InvalidInput),
        };

        self.source(path)
    }

    fn source(&mut self, path: PathBuf) -> Result<(), ErrorKind> {
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return Err(ErrorKind::InvalidInput),
        };

        let reader = std::io::BufReader::new(file);

        use std::io::BufRead;
        for line in reader.lines().flatten() {
            let l = line.trim().to_string();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }

            self.execute(&l)?;
        }

        Ok(())
    }

    pub fn load_login_config(&mut self) {
        let path = self.home_dir.join(".wpcsh_profile");
        let _ = self.source(path);
    }

    pub fn load_interactive_config(&mut self) {
        let path = self.home_dir.join(".wpcshrc");
        let _ = self.source(path);
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
    ) -> Result<std::process::Output, ErrorKind> {
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
        let arg = arg.replace("~", &self.home_dir.to_string_lossy());

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

        if std::env::set_current_dir(new_dir.clone()).is_err() {
            return Err(ErrorKind::InvalidInput);
        }

        if new_dir.is_dir() {
            self.current_dir = new_dir.clone();
            self.variables
                .insert("PWD".to_string(), new_dir.to_string_lossy().to_string());
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
            self.exit_status = ExitStatus::from_raw(0);
        } else {
            self.exit_status = ExitStatus::from_raw(1);
        }
    }

    fn add_alias(&mut self, text: &str) {
        if let Some((key, val)) = text.split_once('=') {
            let val = val.trim_matches('"');
            self.aliases.insert(key.trim().to_string(), val.to_string());
            self.exit_status = ExitStatus::from_raw(0);
        } else {
            self.exit_status = ExitStatus::from_raw(1);
        }
    }

    fn get_prompt(&mut self) -> String {
        if let Some(cmd) = self.variables.get("PROMPT") {
            if let Ok(mut parsed) = self.parse_command(cmd) {
                if let Ok(out) = self.get_result_of_external_command(&mut parsed) {
                    return String::from_utf8_lossy(&out.stdout).to_string();
                }
            }
        }

        format!("{} > ", self.current_dir.display())
    }

    pub fn run_non_interactive(&mut self) {
        use std::io::{self, BufRead};

        let stdin = io::stdin();
        for line in stdin.lock().lines().flatten() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Err(_) = self.execute(line) {
                break;
            }
        }
    }

    pub fn run_interactive(&mut self) {
        use linefeed::{Interface, ReadResult, Signal};

        self.load_interactive_config();

        let interface = Interface::new("wpcsh").expect("no tty");

        let history_path = self.home_dir.join(".wpcsh_history");
        let _ = interface.load_history(&history_path);

        loop {
            let prompt = self.get_prompt();

            if interface.set_prompt(&prompt).is_err() {
                interface.set_prompt(">").expect("Failed to set prompt");
            };

            match interface.read_line() {
                Ok(ReadResult::Input(line)) => {
                    interface.add_history(line.clone());

                    if let Err(err) = self.execute(&line) {
                        match err {
                            ErrorKind::InvalidInput => {
                                eprintln!("wpcsh: invalid input: {}", line);
                            }
                            ErrorKind::NotFound => {
                                eprintln!("wpcsh: command not found: {}", line);
                            }
                            ErrorKind::Interrupted => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(ReadResult::Signal(Signal::Interrupt | Signal::Quit)) => break,
                Ok(ReadResult::Eof) => break,
                _ => {}
            }

            let _ = interface.save_history(&history_path);
        }
    }

    fn clear_terminal(&mut self) -> Result<(), ErrorKind> {
        print!("\x1B[2J\x1B[1;1H");
        use std::io::Write;
        match std::io::stdout().flush() {
            Ok(_) => {
                self.exit_status = ExitStatus::from_raw(0);
                Ok(())
            }
            Err(_) => {
                self.exit_status = ExitStatus::from_raw(1);
                Err(ErrorKind::InvalidInput)
            }
        }
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        use std::path::Component;
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

#[cfg(windows)]
fn get_coreutils_commands() -> std::io::Result<Vec<String>> {
    let output = Command::new("coreutils").arg("--list").output()?;

    let coreutils_commands = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let coreutils_commands = coreutils_commands
        .split_whitespace()
        .map(String::from)
        .collect::<Vec<String>>();

    Ok(coreutils_commands)
}
