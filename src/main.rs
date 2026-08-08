use std::process::ExitCode;

use palc::{Parser, Subcommand};

#[derive(Debug, Parser, Clone)]
#[command(version, long_about)]
struct Cli {
    #[command(subcommand)]
    command: Subcommand,
}

#[derive(Debug, Subcommand, Clone)]
enum Subcommand {
    /// Set a var in the Windows environment variable (as REG_SZ).
    Set { var: String, value: String },
    /// Set a var as REG_EXPAND_SZ so that %VAR% placeholders inside the value
    /// are expanded by consumers.
    SetExpandString { var: String, value: String },
    /// Get a var from the Windows environment variable.
    Get { var: String },
    /// Remove a var from the Windows environment variable.
    Remove { var: String },
    /// Check if a value exists in the Windows environment variable list.
    /// Exits with code 1 if the value does not exist.
    Exists { var: String, value: String },
    /// Append a value at the end to the Windows environment variable list
    Append { var: String, value: String },
    /// Prepend a value at the beginning to the Windows environment variable
    /// list
    Prepend { var: String, value: String },
    /// Remove a value from the Windows environment variable list
    RemoveFromList { var: String, value: String },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> windows_env::Result<ExitCode> {
    match Cli::parse().command {
        Subcommand::Set { var, value } => {
            windows_env::set(&var, &value)?;
            println!("{var}={value}");
        }

        Subcommand::SetExpandString { var, value } => {
            windows_env::set_expand_string(&var, &value)?;
            println!("{var}={value} (REG_EXPAND_SZ)");
        }

        Subcommand::Get { var } => match windows_env::get(&var)? {
            Some(value) => println!("{value}"),
            None => {
                eprintln!("{var} not found");
                return Ok(ExitCode::FAILURE);
            }
        },

        Subcommand::Remove { var } => {
            windows_env::remove(&var)?;
            println!("{var} removed");
        }

        Subcommand::Exists { var, value } => {
            let exists = windows_env::exists_in_list(&var, &value)?;
            println!("{exists}");
            if !exists {
                return Ok(ExitCode::FAILURE);
            }
        }

        Subcommand::Append { var, value } => {
            windows_env::append(&var, &value)?;
            println!("appended: {value} to {var}");
        }

        Subcommand::Prepend { var, value } => {
            windows_env::prepend(&var, &value)?;
            println!("prepended: {value} to {var}");
        }

        Subcommand::RemoveFromList { var, value } => {
            if windows_env::remove_from_list(&var, &value)? {
                println!("removed: {value} from {var}");
            } else {
                eprintln!("{value} not found in {var}");
                return Ok(ExitCode::FAILURE);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
