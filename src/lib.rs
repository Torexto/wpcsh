mod flash;
mod token;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::ops::Deref;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};

#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;

use crate::flash::parser::{Node, Redirect, RedirectKind};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

const BUILTINS: &[&str] = &["cd", "exit", "export", "alias", "source", "clear"];

fn is_builtin(command: &str) -> bool {
    BUILTINS.contains(&command)
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

#[derive(Debug)]
struct CommandContainer {
    program: String,
    args: Vec<String>,
}

impl CommandContainer {
    fn new(program: String, args: Vec<String>) -> Self {
        Self { program, args }
    }
}

fn apply_redirect(command: &mut Command, kind: &RedirectKind, target: &str) -> std::io::Result<()> {
    match kind {
        RedirectKind::Input => {
            let file = File::open(target)?;
            command.stdin(Stdio::from(file));
        }
        RedirectKind::Output => {
            let file = File::create(target)?;
            command.stdout(Stdio::from(file));
        }
        RedirectKind::Append => {
            let file = OpenOptions::new().append(true).create(true).open(target)?;
            command.stdout(Stdio::from(file));
        }
        RedirectKind::HereDoc | RedirectKind::HereDocDash => {
            unimplemented!();
            // let (mut reader, mut writer) = os_pipe::pipe()?;
            // writer.write_all(target.as_bytes())?;
            // drop(writer);
            // command.stdin(Stdio::from(reader));
        }
        RedirectKind::HereString => {
            unimplemented!();
            // let (mut reader, mut writer) = os_pipe::pipe()?;
            // writer.write_all(target.as_bytes())?;
            // drop(writer);
            // command.stdin(Stdio::from(reader));
        }
        RedirectKind::InputDup | RedirectKind::OutputDup => {
            // tutaj trzeba użyć unsafe i dup2 na Unixie, na Windows użyj handli
            unimplemented!()
        }
    }
    Ok(())
}

impl Shell {
    pub fn execute(&mut self, buffer: &str) -> Result<i32, ErrorKind> {
        let lexer = flash::lexer::Lexer::new(buffer);
        let mut parser = flash::parser::Parser::new(lexer);
        let statement = parser.parse_command();

        #[cfg(debug_assertions)]
        dbg!(&statement);

        match statement {
            Node::Command {
                name,
                args,
                redirects,
            } => {
                let (name, args) = self.resolve_alias(Cow::Owned(name), args);

                if is_builtin(&name) {
                    self.execute_command(&mut CommandContainer::new(name, args))
                } else {
                    let mut command = Command::new(name);
                    command.envs(self.variables.iter()).args(args);

                    for redirect in redirects.into_iter() {
                        apply_redirect(&mut command, &redirect.kind, &redirect.file)
                            .expect("Failed to apply redirect");
                    }

                    let status = command
                        .spawn()
                        .and_then(|mut c| c.wait())
                        .expect("Failed to spawn child process");
                    Ok(status.code().expect("Failed to get exit code"))
                }
            }
            Node::Pipeline { commands } => {
                let mut previous_stdout: Option<Stdio> = None;
                let mut childrens: Vec<Child> = Vec::new();
                let length = commands.len();

                for (i, command) in commands.into_iter().enumerate() {
                    if let Node::Command {
                        name,
                        args,
                        redirects,
                    } = command
                    {
                        let (name, args) = self.resolve_alias(Cow::Owned(name), args);

                        let mut command = Command::new(name);
                        command.envs(self.variables.iter()).args(args);

                        if let Some(stdin) = previous_stdout.take() {
                            command.stdin(stdin);
                        }

                        let is_last = i == length - 1;

                        if !is_last {
                            command.stdout(Stdio::piped());
                        } else {
                            command.stdout(Stdio::inherit());
                        }

                        for redirect in redirects.into_iter() {
                            apply_redirect(&mut command, &redirect.kind, &redirect.file)
                                .expect("Failed to apply redirect");
                        }

                        let mut child = command.spawn().expect("Failed to spawn child process");

                        if !is_last {
                            previous_stdout = Some(child.stdout.take().unwrap().into())
                        }

                        childrens.push(child);
                    }
                }

                let mut last_code = 0;
                for mut child in childrens {
                    let status = child.wait().ok();
                    if let Some(code) = status.and_then(|s| s.code()) {
                        last_code = code;
                    }
                }

                Ok(last_code)
            }
            Node::List {
                statements,
                operators,
            } => {
                println!("{:?}", statements);
                println!("{:?}", operators);
                unimplemented!()
            }
            Node::Assignment { .. } => {
                unimplemented!()
            }
            Node::CommandSubstitution { .. } => {
                unimplemented!()
            }
            Node::ArithmeticExpansion { .. } => {
                unimplemented!()
            }
            Node::ArithmeticCommand { .. } => {
                unimplemented!()
            }
            Node::Subshell { .. } => {
                unimplemented!()
            }
            Node::Comment(_) => {
                unimplemented!()
            }
            Node::StringLiteral(_) => {
                unimplemented!()
            }
            Node::SingleQuotedString(_) => {
                unimplemented!()
            }
            Node::ExtGlobPattern { .. } => {
                unimplemented!()
            }
            Node::IfStatement { .. } => {
                unimplemented!()
            }
            Node::ElifBranch { .. } => {
                unimplemented!()
            }
            Node::ElseBranch { .. } => {
                unimplemented!()
            }
            Node::CaseStatement { .. } => {
                unimplemented!()
            }
            Node::Array { .. } => {
                unimplemented!()
            }
            Node::Function { .. } => {
                unimplemented!()
            }
            Node::FunctionCall { .. } => {
                unimplemented!()
            }
            Node::Export { name, value } => {
                match value.unwrap().deref() {
                    Node::StringLiteral(value) => self.add_variable(&format!("{}={}", name, value)),
                    _ => {}
                };
                Ok(0)
            }
            Node::Return { .. } => {
                unimplemented!()
            }
            Node::ExtendedTest { .. } => {
                unimplemented!()
            }
            Node::HistoryExpansion { .. } => {
                unimplemented!()
            }
            Node::Complete { .. } => {
                unimplemented!()
            }
            Node::ForLoop { .. } => {
                unimplemented!()
            }
            Node::WhileLoop { .. } => {
                unimplemented!()
            }
            Node::UntilLoop { .. } => {
                unimplemented!()
            }
            Node::Negation { .. } => {
                unimplemented!()
            }
            Node::SelectStatement { .. } => {
                unimplemented!()
            }
            Node::Group { .. } => {
                unimplemented!()
            }
            Node::ParameterExpansion { .. } => {
                unimplemented!()
            }
            Node::ProcessSubstitution { .. } => {
                unimplemented!()
            }
        }
    }

    fn execute_command(&mut self, command: &mut CommandContainer) -> Result<i32, ErrorKind> {
        let _ = match command.program.as_str() {
            "clear" => self.clear_terminal(),
            "cd" => self.change_directory(&command.args),
            "export" => {
                self.add_variable(&command.args.join(" "));
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
            _ => unreachable!()
        };

        Ok(0)
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
        let file = match File::open(&path) {
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
        name: String,
        args: Vec<String>,
        redirects: Vec<Redirect>,
    ) -> Result<std::process::Output, ErrorKind> {
        let (name, args) = self.resolve_alias(Cow::Owned(name), args);

        let mut command = Command::new(name);
        command.envs(self.variables.iter()).args(args);

        for redirect in redirects.into_iter() {
            apply_redirect(&mut command, &redirect.kind, &redirect.file)
                .expect("Failed to apply redirect");
        }

        let status = command.output().expect("Failed to execute child process");
        Ok(status)
    }

    fn resolve_alias(&self, cmd: Cow<String>, args: Vec<String>) -> (String, Vec<String>) {
        let alias = self.aliases.get(cmd.as_ref()).unwrap_or(cmd.as_ref());
        let mut split = alias.split_whitespace();
        let name = split.next().unwrap_or(cmd.as_ref()).to_string();
        let mut argv = split.map(String::from).collect::<Vec<String>>();
        argv.extend(args);

        (name, argv)
    }

    // fn resolve_alias(&self, command: &str) -> Vec<String> {
    //     let mut tokens = vec![command.to_owned()];
    //     let mut seen = std::collections::HashSet::new();
    //
    //     while let Some(alias) = self.aliases.get(&tokens[0]) {
    //         if !seen.insert(tokens[0].clone()) || seen.len() > 32 {
    //             break;
    //         }
    //
    //         let mut alias_tokens = match tokenize(alias) {
    //             Ok(tokens) => tokens,
    //             Err(_) => continue,
    //         };
    //
    //         alias_tokens.extend(tokens.drain(1..));
    //         tokens = alias_tokens;
    //     }
    //
    //     tokens
    // }

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
        let t = &self.variables.get("PROMPT");
        if let Some(cmd) = self.variables.get("PROMPT") {
            let lexer = flash::lexer::Lexer::new(cmd);
            let mut parser = flash::parser::Parser::new(lexer);

            let node = parser.parse_command();

            if let Node::Command {
                name,
                args,
                redirects,
            } = node
            {
                dbg!(&name, &args, &redirects);
                if let Ok(out) = self.get_result_of_external_command(name, args, redirects) {
                    return String::from_utf8_lossy(&out.stdout).to_string();
                }
            }
        } else {
            dbg!("PROMPT not set");
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
                    std::io::stdout().flush().unwrap();
                    println!();
                }
                Ok(ReadResult::Signal(Signal::Quit)) => break,
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
