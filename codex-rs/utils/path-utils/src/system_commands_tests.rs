use super::*;
use pretty_assertions::assert_eq;

#[test]
fn termux_prefix_must_be_the_usr_directory_of_an_android_app() {
    assert!(is_termux_prefix(Path::new(
        "/data/data/com.termux/files/usr"
    )));
    assert!(is_termux_prefix(Path::new(
        "/data/data/com.rebroad.termux/files/usr"
    )));
    assert!(is_termux_prefix(Path::new(
        "/data/data/org.termux/files/usr"
    )));
    assert!(!is_termux_prefix(Path::new(
        "/data/data/com.termux/files/home"
    )));
    assert!(!is_termux_prefix(Path::new("/tmp/usr")));
}

#[cfg(target_os = "android")]
#[test]
fn searches_the_termux_prefix_for_installed_helpers() {
    let prefix = termux_prefix().expect("Termux PREFIX should resolve to its app usr directory");
    assert!(system_directories().contains(&prefix.join("bin")));
    assert!(installation_roots().contains(&prefix));

    for helper in ["termux-clipboard-set", "tmux"] {
        let executable = prefix.join("bin").join(helper);
        if executable.is_file() {
            assert_eq!(
                system_executable(helper),
                dunce::canonicalize(executable).ok()
            );
        }
    }
}

#[test]
fn resolves_only_executables_inside_installation_directories() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join(format!("helper{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&executable, "installed helper").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(/*mode*/ 0o755))
            .unwrap();
    }
    let directories = vec![dunce::canonicalize(bin).unwrap()];
    assert_eq!(
        executable_in_directories("helper", &directories),
        Some(dunce::canonicalize(&executable).unwrap())
    );
    assert_eq!(executable_in_directories("../helper", &directories), None);
    assert_eq!(executable_in_directories("missing", &directories), None);
    #[cfg(unix)]
    {
        let outside_root = if cfg!(target_os = "android") {
            let home = std::env::var_os("HOME").expect("Termux sets HOME");
            tempfile::tempdir_in(home).unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let outside = outside_root.path().join("outside");
        std::fs::rename(&executable, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &executable).unwrap();
        assert_eq!(executable_in_directories("helper", &directories), None);
    }
}
