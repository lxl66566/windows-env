use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser, Clone)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// A var name and its value.
/// options.
#[derive(Debug, Args, Clone)]
struct VarValue {
    var: String,
    #[arg(allow_hyphen_values = true)]
    value: String,
}

#[derive(Debug, Subcommand, Clone)]
enum Commands {
    /// Set a var in the Windows environment variable (as REG_SZ).
    Set(VarValue),
    /// Set a var as REG_EXPAND_SZ so that %VAR% placeholders inside the value
    /// are expanded by consumers.
    SetExpandString(VarValue),
    /// Get a var from the Windows environment variable.
    Get { var: String },
    /// Remove a var from the Windows environment variable.
    Remove { var: String },
    /// Check if a value exists in the Windows environment variable list.
    /// Exits with code 1 if the value does not exist.
    Exists(VarValue),
    /// Append a value at the end to the Windows environment variable list
    Append(VarValue),
    /// Prepend a value at the beginning to the Windows environment variable
    /// list
    Prepend(VarValue),
    /// Remove a value from the Windows environment variable list
    RemoveFromList(VarValue),
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        },
    }
}

fn run() -> windows_env::Result<ExitCode> {
    match Cli::parse().command {
        Commands::Set(VarValue { var, value }) => {
            windows_env::set(&var, &value)?;
            println!("{var}={value}");
        },

        Commands::SetExpandString(VarValue { var, value }) => {
            windows_env::set_expand_string(&var, &value)?;
            println!("{var}={value} (REG_EXPAND_SZ)");
        },

        Commands::Get { var } => {
            if let Some(value) = windows_env::get(&var)? {
                println!("{value}");
            } else {
                eprintln!("{var} not found");
                return Ok(ExitCode::FAILURE);
            }
        },

        Commands::Remove { var } => {
            windows_env::remove(&var)?;
            println!("{var} removed");
        },

        Commands::Exists(VarValue { var, value }) => {
            let exists = windows_env::exists_in_list(&var, &value)?;
            println!("{exists}");
            if !exists {
                return Ok(ExitCode::FAILURE);
            }
        },

        Commands::Append(VarValue { var, value }) => {
            windows_env::append(&var, &value)?;
            println!("appended: {value} to {var}");
        },

        Commands::Prepend(VarValue { var, value }) => {
            windows_env::prepend(&var, &value)?;
            println!("prepended: {value} to {var}");
        },

        Commands::RemoveFromList(VarValue { var, value }) => {
            if windows_env::remove_from_list(&var, &value)? {
                println!("removed: {value} from {var}");
            } else {
                eprintln!("{value} not found in {var}");
                return Ok(ExitCode::FAILURE);
            }
        },
    }
    Ok(ExitCode::SUCCESS)
}
