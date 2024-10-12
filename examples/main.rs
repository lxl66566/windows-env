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
