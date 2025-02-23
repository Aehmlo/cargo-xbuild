use std::io::{self, Write};
use std::mem;
use std::path::Path;
use std::path::{Display, PathBuf};
use std::process::{Command, ExitStatus};
use std::env;

use rustc_version::VersionMeta;

use cargo::Rustflags;
use cli::Args;
use config::Config;
use errors::*;
use extensions::CommandExt;
use flock::{FileLock, Filesystem};
use CompilationMode;

pub fn run(
    args: &Args,
    cmode: &CompilationMode,
    rustflags: Rustflags,
    home: &Home,
    meta: &VersionMeta,
    command_name: &str,
    verbose: bool,
) -> Result<ExitStatus> {
    let cargo = std::env::var("CARGO").unwrap_or("cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.arg(command_name);
    cmd.args(args.all());

    let flags = rustflags.for_xargo(home)?;
    if verbose {
        writeln!(io::stderr(), "+ RUSTFLAGS={:?}", flags).ok();
    }
    cmd.env("RUSTFLAGS", flags);

    let locks = (home.lock_ro(&meta.host), home.lock_ro(cmode.triple()));

    let status = cmd.run_and_get_status(verbose)?;

    mem::drop(locks);

    Ok(status)
}

pub struct Home {
    path: Filesystem,
}

impl Home {
    pub fn display(&self) -> Display {
        self.path.display()
    }

    fn path(&self, triple: &str) -> Filesystem {
        self.path.join("lib").join("rustlib").join(triple)
    }

    pub fn lock_ro(&self, triple: &str) -> Result<FileLock> {
        let fs = self.path(triple);

        fs.open_ro(".sentinel", &format!("{}'s sysroot", triple))
            .chain_err(|| format!("couldn't lock {}'s sysroot as read-only", triple))
    }

    pub fn lock_rw(&self, triple: &str) -> Result<FileLock> {
        let fs = self.path(triple);

        fs.open_rw(".sentinel", &format!("{}'s sysroot", triple))
            .chain_err(|| format!("couldn't lock {}'s sysroot in {} as read-write", triple, fs.display()))
    }
}

pub fn home(root: &Path, config: &Config) -> Result<Home> {
    let path = if let Ok(path) = env::var("XBUILD_SYSROOT_PATH") {
        PathBuf::from(path)
    } else {
        let mut path = PathBuf::from(root);
        path.push(&config.sysroot_path);
        path
    };

    Ok(Home {
        path: Filesystem::new(path),
    })
}
