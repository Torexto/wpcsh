use crate::{path_to_str, State};

pub fn pwd(state: &State) -> Result<(), String> {
    println!("{}", path_to_str(&state.current_dir));
    
    Ok(())
}