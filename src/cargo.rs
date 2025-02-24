use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::{env, fmt};

use toml::Value;

use cli::Args;
use errors::*;
use extensions::CommandExt;
use util;
use xargo::Home;

pub struct Rustflags {
    flags: Vec<String>,
}

impl Rustflags {
    pub fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        let mut flags = self.flags.iter();

        while let Some(flag) = flags.next() {
            if flag == "-C" {
                if let Some(next) = flags.next() {
                    if next.starts_with("link-arg=") || next.starts_with("link-args=") {
                        // don't hash linker arguments
                    } else {
                        flag.hash(hasher);
                        next.hash(hasher);
                    }
                } else {
                    flag.hash(hasher);
                }
            } else {
                flag.hash(hasher);
            }
        }
    }

    /// Stringifies these flags for Xargo consumption
    pub fn for_xargo(&self, home: &Home) -> Result<String> {
        let sysroot = format!("{}", home.display());
        if env::var_os("XBUILD_ALLOW_SYSROOT_SPACES").is_none() && sysroot.contains(" ") {
            return Err(format!("Sysroot must not contain spaces!\n\
            See issue https://github.com/rust-lang/cargo/issues/6139\n\n\
            The sysroot is `{}`.\n\n\
            To override this error, you can set the `XBUILD_ALLOW_SYSROOT_SPACES`\
            environment variable.", sysroot).into());
        }
        let mut flags = self.flags.clone();
        flags.push("--sysroot".to_owned());
        flags.push(sysroot);
        Ok(flags.join(" "))
    }
}

impl fmt::Display for Rustflags {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.flags.join(" "), f)
    }
}

pub fn rustflags(config: Option<&Config>, target: &str) -> Result<Rustflags> {
    flags(config, target, "rustflags").map(|fs| Rustflags { flags: fs })
}

/// Returns the flags for `tool` (e.g. rustflags)
///
/// This looks into the environment and into `.cargo/config`
fn flags(config: Option<&Config>, target: &str, tool: &str) -> Result<Vec<String>> {
    if let Some(t) = env::var_os(tool.to_uppercase()) {
        return Ok(t
            .to_string_lossy()
            .split_whitespace()
            .map(|w| w.to_owned())
            .collect());
    }

    if let Some(config) = config.as_ref() {
        let mut build = false;
        if let Some(array) = config
            .table
            .lookup(&format!("target.{}.{}", target, tool))
            .or_else(|| {
                build = true;
                config.table.lookup(&format!("build.{}", tool))
            })
        {
            let mut flags = vec![];

            let mut error = false;
            if let Some(array) = array.as_slice() {
                for value in array {
                    if let Some(flag) = value.as_str() {
                        flags.push(flag.to_owned());
                    } else {
                        error = true;
                        break;
                    }
                }
            } else {
                error = true;
            }

            if error {
                if build {
                    Err(format!(
                        ".cargo/config: build.{} must be an array \
                         of strings",
                        tool
                    ))?
                } else {
                    Err(format!(
                        ".cargo/config: target.{}.{} must be an \
                         array of strings",
                        target, tool
                    ))?
                }
            } else {
                Ok(flags)
            }
        } else {
            Ok(vec![])
        }
    } else {
        Ok(vec![])
    }
}

pub fn run(args: &Args, verbose: bool) -> Result<ExitStatus> {
    let cargo = std::env::var("CARGO").unwrap_or("cargo".to_string());
    Command::new(cargo)
        .arg("build")
        .args(args.all())
        .run_and_get_status(verbose)
}

#[derive(Debug)]
pub struct Config {
    parent_path: PathBuf,
    table: Value,
}

impl Config {
    pub fn target(&self) -> Result<Option<String>> {
        if let Some(v) = self.table.lookup("build.target") {
            let target = v
                .as_str()
                .ok_or_else(|| format!(".cargo/config: build.target must be a string"))?;
            if target.ends_with(".json") {
                let target_path = self.parent_path.join(target);
                let canonicalized = target_path.canonicalize().map_err(|err| {
                    format!(
                        "target JSON file {} does not exist: {}",
                        target_path.display(),
                        err
                    )
                })?;
                let as_string = canonicalized
                    .into_os_string()
                    .into_string()
                    .map_err(|err| format!("target path not valid utf8: {:?}", err))?;
                Ok(Some(as_string))
            } else {
                Ok(Some(target.to_owned()))
            }
        } else {
            Ok(None)
        }
    }
}

pub fn config() -> Result<Option<Config>> {
    let cd = env::current_dir().chain_err(|| "couldn't get the current directory")?;

    if let Some(p) = util::search(&cd, ".cargo/config") {
        Ok(Some(Config {
            parent_path: p.to_owned(),
            table: util::parse(&p.join(".cargo/config"))?,
        }))
    } else {
        Ok(None)
    }
}

pub struct Profile<'t> {
    table: &'t Value,
}

impl<'t> Profile<'t> {
    pub fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        let mut v = self.table.clone();

        // Don't include `lto` in the hash because it doesn't affect compilation
        // of `.rlib`s
        if let Value::Table(ref mut table) = v {
            table.remove("lto");

            // don't hash an empty map
            if table.is_empty() {
                return;
            }
        }

        v.to_string().hash(hasher);
    }
}

impl<'t> fmt::Display for Profile<'t> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut map = BTreeMap::new();
        map.insert("profile".to_owned(), {
            let mut map = BTreeMap::new();
            map.insert("release".to_owned(), self.table.clone());
            Value::Table(map)
        });

        fmt::Display::fmt(&Value::Table(map), f)
    }
}

pub struct Toml {
    table: Value,
}

impl Toml {
    /// `profile.release` part of `Cargo.toml`
    pub fn profile(&self) -> Option<Profile> {
        self.table
            .lookup("profile.release")
            .map(|t| Profile { table: t })
    }
}

pub fn toml(root: &Path) -> Result<Toml> {
    util::parse(&root.join("Cargo.toml")).map(|t| Toml { table: t })
}
