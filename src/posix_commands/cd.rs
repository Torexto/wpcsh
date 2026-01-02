use crate::{normalize_path, State};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;

pub fn cd(state: &mut State, args: Vec<String>) -> Result<(), String> {
    if args.len() > 1 {
        return Err("wpcsh: cd: too many arguments".to_string());
    }

    let path = args.get(0);
    match path {
        Some(path) => {
            let path = path.replace("~", state.home_dir.to_str().unwrap_or(""));
            let new_path = normalize_path(state.current_dir.join(&path));
            if new_path.is_dir() {
                state.current_dir = new_path;
                state.exit_status = ExitStatus::from_raw(0);
            } else {
                state.exit_status = ExitStatus::from_raw(1);
                return Err(format!("cd: no such file or directory: {}", path))
            }
        }
        None => state.current_dir = state.home_dir.clone(),
    };

    Ok(())
}
