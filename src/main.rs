use palc::{Parser, Subcommand};

#[derive(Debug, Parser, Clone)]
#[command(version, long_about)]
struct Cli {
    #[command(subcommand)]
    command: Subcommand,
}

#[derive(Debug, Subcommand, Clone)]
enum Subcommand {
    /// Set a var in the Windows environment variable.
    Set { var: String, value: String },
    /// Get the current Windows environment variable RegKey.
    Get { var: String },
    /// Remove a var from the Windows environment variable.
    Remove { var: String },
    /// Check if a value exists in the Windows environment variable list
    Exists { var: String, value: String },
    /// Append a value at the end to the Windows environment variable list
    Append { var: String, value: String },
    /// Prepend a value at the beginning to the Windows environment variable
    /// list
    Prepend { var: String, value: String },
    /// Remove a value from the Windows environment variable list
    RemoveFromList { var: String, value: String },
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Subcommand::Set { var, value } => {
            windows_env::set(&var, &value)?;
            println!("{}={}", var, value);
        }

        Subcommand::Get { var } => {
            let value = windows_env::get(&var)?.unwrap_or_else(|| panic!("{} not found", var));
            println!("{}", value);
        }

        Subcommand::Remove { var } => {
            windows_env::remove(&var)?;
            println!("{} removed", var);
        }

        Subcommand::Exists { var, value } => {
            let exists = windows_env::exists_in_list(&var, &value)?;
            println!("{}", exists);
        }

        Subcommand::Append { var, value } => {
            windows_env::append(&var, &value)?;
            println!("appended: {} to {}", value, var);
        }

        Subcommand::Prepend { var, value } => {
            windows_env::prepend(&var, &value)?;
            println!("prepended: {} to {}", value, var);
        }

        Subcommand::RemoveFromList { var, value } => {
            windows_env::remove_from_list(&var, &value)?;
            println!("removed: {} from {}", value, var);
        }
    }
    Ok(())
}
