# windows-env

Easily manage Windows environment variables **permanently**, without the need to restart your system. (**but terminal restart is required**)

Features:

- Easily operate list variables like `PATH`.

## Installation

- as lib:
  ```toml
  [target."cfg(windows)".dependencies]
  windows_env = "0.2.0"
  ```
- as executable binary:
  ```sh
  cargo binstall windows-env        # see cargo-binstall: https://github.com/cargo-bins/cargo-binstall
  cargo install windows-env -F bin  # or compile from source manually
  ```

## Example

- binary usage: runs `wenv -h` to see help message.
- lib usage:

  ```rs
  fn main() -> std::io::Result<()> {
      windows_env::set("TEST_ENV", "test")?;
      assert_eq!(windows_env::get("TEST_ENV")?.unwrap(), "test");
      windows_env::remove("TEST_ENV")?;
      assert!(windows_env::get("TEST_ENV")?.is_none());

      windows_env::append("TEST_ENV", "test1")?;
      windows_env::prepend("TEST_ENV", "test2")?;
      assert_eq!(windows_env::get("TEST_ENV")?.unwrap(), "test2;test1");

      windows_env::remove_from_list("TEST_ENV", "test2")?;
      assert!(windows_env::exists_in_list("TEST_ENV", "test1")?);

      windows_env::remove("TEST_ENV")?;
      Ok(())
  }
  ```

  - using windows-env as a lib will affect the current process, so you can use the new env by spawning processes after modifying env in rust code.

## Compare

- [set_env](https://crates.io/crates/set_env):
  - it uses powershell script while this crate uses windows api

## TODO

- [x] cli support
- [ ] System env modification

## MSRV

- v0.1.1: 1.70
