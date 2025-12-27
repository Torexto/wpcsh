use crate::State;
use std::borrow::Cow;

pub fn echo(state: Option<&State>, args: &[&str]) -> Result<(), String> {
    let args = args
        .iter()
        .map(|arg| {
            if !arg.starts_with('$') {
                return Cow::Borrowed(*arg);
            }

            let var_name = &arg[1..];

            match state {
                Some(state) => {
                    if let Some(value) = state.vars.get(var_name) {
                        Cow::Borrowed(value.as_str())
                    } else {
                        Cow::Owned(std::env::var(var_name).unwrap_or_default())
                    }
                }
                None => Cow::Owned(std::env::var(var_name).unwrap_or_default()),
            }
        })
        .collect::<Vec<Cow<str>>>();

    println!("{}", args.join(" "));
    Ok(())
}
