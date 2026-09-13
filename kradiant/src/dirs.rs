//! Helper functions for accessing XDG desktop directories

use std::env;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::sync::LazyLock;

pub fn init() {
    let c = config_home().join("kradiant_editor");
    let d = data_home().join("kradiant_editor").join("themes");

    if !c.exists() {
        let _ = create_dir_all(c);
    }
    if !d.exists() {
        let _ = create_dir_all(d);
    }
}

pub fn user_home() -> &'static PathBuf {
    static HOME_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
        if let Some(h) = env::home_dir() {
            h
        }
        else {
            env::current_exe().unwrap().with_extension(".home")
        }
    });

    &HOME_DIR
}

pub fn config_home() -> &'static PathBuf {
    static CONFIG_HOME: LazyLock<PathBuf> = LazyLock::new(|| {
        if let Ok(s) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(s)
        }
        else if let Some(s) = env::home_dir() {
            s.join(".config")
        }
        else {
            env::current_exe().unwrap().with_extension(".config")
        }
    });

    &CONFIG_HOME
}

pub fn config_dirs() -> &'static Vec<PathBuf> {
    static CONFIG_DIRS: LazyLock<Vec<PathBuf>> = LazyLock::new(|| {
        if let Ok(s) = env::var("XDG_CONFIG_DIRS") {
            s.split(":").filter(|p| *p != "=").map(|p| PathBuf::from(p)).collect()
        }
        else {
            vec![PathBuf::from("/etc/xdg")]
        }
    });

    &CONFIG_DIRS
}

pub fn data_home() -> &'static PathBuf {
    static DATA_HOME: LazyLock<PathBuf> = LazyLock::new(|| {
        if let Ok(s) = env::var("XDG_DATA_HOME") {
            PathBuf::from(s)
        }
        else {
            user_home().join(".local").join("share")
        }
    });

    &DATA_HOME
}

pub fn data_dirs() -> &'static Vec<PathBuf> {
    static DATA_DIRS: LazyLock<Vec<PathBuf>> = LazyLock::new(|| {
        if let Ok(s) = env::var("XDG_DATA_DIRS") {
            s.split(":").filter(|p| *p != "=").map(|p| PathBuf::from(p)).collect()
        }
        else {
            vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]
        }
    });

    &DATA_DIRS
}

pub fn cache_home() -> &'static PathBuf {
    static CACHE_HOME: LazyLock<PathBuf> = LazyLock::new(|| {
        if let Ok(s) = env::var("XDG_CACHE_HOME") {
            PathBuf::from(s)
        }
        else {
            user_home().join(".cache")
        }
    });

    &CACHE_HOME
}
