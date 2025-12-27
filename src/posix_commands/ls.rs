use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use crate::State;

pub fn ls(state: &mut State, args: &[&str]) -> Result<(), String> {
    for entry in state.current_dir.read_dir().unwrap() {
        let entry = entry.unwrap();
        println!("{}", entry.file_name().to_str().unwrap());
    }
    
    state.exit_status = ExitStatus::from_raw(0);
    
    Ok(())
}
