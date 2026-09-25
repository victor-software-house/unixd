use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, error, fmt, fs, io};

use crate::Version;

/// A daemon's identity: its name and protocol major. The label, the unit
/// files, and the socket name all carry the major, so two majors never share
/// a socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    name: String,
    major: u16,
}

impl Service {
    /// The service `name` speaking `version`. `name` becomes a file name, so
    /// it should be a short word such as the program's name.
    pub fn new(name: impl Into<String>, version: Version) -> Self {
        Self {
            name: name.into(),
            major: version.major,
        }
    }

    /// `<name>-v<major>`: the launchd label, the systemd unit name, and the
    /// socket's file name.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}-v{}", self.name, self.major)
    }

    /// Where the service manager listens: under
    /// `~/Library/Application Support/<name>/` on macOS, and under
    /// `$XDG_RUNTIME_DIR/<name>/` on Linux.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoBaseDirectory`] when `HOME` or `XDG_RUNTIME_DIR` is
    /// unset or relative.
    pub fn socket_path(&self) -> Result<PathBuf, InstallError> {
        Ok(platform::socket_dir(&self.name)?.join(format!("{}.sock", self.label())))
    }
}

/// Why [`install`] or [`uninstall`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum InstallError {
    /// A needed base directory is unset or relative.
    NoBaseDirectory(&'static str),
    /// The program or an argument is not UTF-8, which a `LaunchAgent` cannot
    /// hold.
    NotUnicode(OsString),
    /// The socket path is longer than the platform allows.
    SocketPathTooLong {
        /// The path that is too long.
        path: PathBuf,
        /// The most bytes the platform allows.
        limit: usize,
    },
    /// `launchctl` or `systemctl` failed.
    Command {
        /// The command line.
        command: String,
        /// Its standard error.
        stderr: String,
    },
    /// A unit file could not be written or removed.
    Io(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseDirectory(variable) => {
                write!(formatter, "{variable} is unset or not an absolute path")
            }
            Self::NotUnicode(word) => write!(formatter, "{} is not UTF-8", word.display()),
            Self::SocketPathTooLong { path, limit } => write!(
                formatter,
                "the socket path {} is longer than {limit} bytes",
                path.display()
            ),
            Self::Command { command, stderr } => write!(formatter, "{command} failed: {stderr}"),
            Self::Io(error) => write!(formatter, "unit file error: {error}"),
        }
    }
}

impl error::Error for InstallError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Installs `service` so the service manager listens on its socket and
/// starts `program` with `args` on the first connection. Running it again
/// replaces the units with the same content and reloads them.
///
/// # Errors
///
/// [`InstallError::SocketPathTooLong`] before any file is written, and the
/// other variants when a file or the service manager fails.
pub fn install(service: &Service, program: &Path, args: &[OsString]) -> Result<(), InstallError> {
    let socket = service.socket_path()?;
    let length = socket.as_os_str().len();
    if length > platform::MAX_SOCKET_PATH {
        return Err(InstallError::SocketPathTooLong {
            path: socket,
            limit: platform::MAX_SOCKET_PATH,
        });
    }
    platform::install(service, &socket, program, args)
}

/// Stops `service` and removes its units and its socket. Removing a service
/// that is not installed succeeds.
///
/// # Errors
///
/// When a file or the service manager fails.
pub fn uninstall(service: &Service) -> Result<(), InstallError> {
    platform::uninstall(service)
}

fn absolute_env(variable: &'static str) -> Result<PathBuf, InstallError> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(InstallError::NoBaseDirectory(variable))
}

fn run(command: &mut Command) -> Result<(), InstallError> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(InstallError::Command {
        command: format!("{command:?}"),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde::Serialize;

    use super::{InstallError, Service, absolute_env, remove_if_present, run};

    /// `sizeof(sun_path)` is 104, and the path needs its terminating NUL.
    pub(super) const MAX_SOCKET_PATH: usize = 103;

    #[derive(Serialize)]
    struct Agent<'a> {
        #[serde(rename = "Label")]
        label: &'a str,
        #[serde(rename = "ProgramArguments")]
        program_arguments: Vec<&'a str>,
        #[serde(rename = "Sockets")]
        sockets: Sockets<'a>,
        #[serde(rename = "inetdCompatibility")]
        inetd_compatibility: Inetd,
        #[serde(rename = "ThrottleInterval")]
        throttle_interval: u32,
    }

    #[derive(Serialize)]
    struct Sockets<'a> {
        #[serde(rename = "Listener")]
        listener: Listener<'a>,
    }

    #[derive(Serialize)]
    struct Listener<'a> {
        #[serde(rename = "SockPathName")]
        path: &'a Path,
        #[serde(rename = "SockPathMode")]
        mode: u32,
    }

    #[derive(Serialize)]
    struct Inetd {
        #[serde(rename = "Wait")]
        wait: bool,
    }

    /// Creates `path` and its parents as `0700`, as the socket's directory
    /// must be. launchd creates the socket but not its directory.
    fn private_dir(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }

    pub(super) fn socket_dir(name: &str) -> Result<PathBuf, InstallError> {
        Ok(absolute_env("HOME")?
            .join("Library")
            .join("Application Support")
            .join(name))
    }

    fn plist_path(service: &Service) -> Result<PathBuf, InstallError> {
        Ok(absolute_env("HOME")?
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", service.label())))
    }

    fn domain() -> String {
        format!("gui/{}", rustix::process::geteuid().as_raw())
    }

    fn loaded(service: &Service) -> bool {
        Command::new("launchctl")
            .arg("print")
            .arg(format!("{}/{}", domain(), service.label()))
            .output()
            .is_ok_and(|output| output.status.success())
    }

    pub(super) fn install(
        service: &Service,
        socket: &Path,
        program: &Path,
        args: &[OsString],
    ) -> Result<(), InstallError> {
        let label = service.label();
        let program_arguments = std::iter::once(program.as_os_str())
            .chain(args.iter().map(OsString::as_os_str))
            .map(|word| {
                word.to_str()
                    .ok_or_else(|| InstallError::NotUnicode(word.to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let agent = Agent {
            label: &label,
            program_arguments,
            sockets: Sockets {
                listener: Listener {
                    path: socket,
                    mode: 0o600,
                },
            },
            inetd_compatibility: Inetd { wait: true },
            throttle_interval: 1,
        };
        if let Some(dir) = socket.parent() {
            private_dir(dir)?;
        }
        let plist = plist_path(service)?;
        if let Some(dir) = plist.parent() {
            std::fs::create_dir_all(dir)?;
        }
        plist::to_file_xml(&plist, &agent).map_err(|error| {
            InstallError::Io(std::io::Error::other(error.to_string()))
        })?;
        if loaded(service) {
            run(Command::new("launchctl")
                .arg("bootout")
                .arg(format!("{}/{label}", domain())))?;
        }
        run(Command::new("launchctl")
            .arg("bootstrap")
            .arg(domain())
            .arg(&plist))
    }

    pub(super) fn uninstall(service: &Service) -> Result<(), InstallError> {
        if loaded(service) {
            run(Command::new("launchctl")
                .arg("bootout")
                .arg(format!("{}/{}", domain(), service.label())))?;
        }
        remove_if_present(&plist_path(service)?)?;
        remove_if_present(&service.socket_path()?)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use indoc::formatdoc;

    use super::{InstallError, Service, absolute_env, remove_if_present, run};

    /// `sizeof(sun_path)` is 108, and the path needs its terminating NUL.
    pub(super) const MAX_SOCKET_PATH: usize = 107;

    pub(super) fn socket_dir(name: &str) -> Result<PathBuf, InstallError> {
        Ok(absolute_env("XDG_RUNTIME_DIR")?.join(name))
    }

    fn unit_dir() -> Result<PathBuf, InstallError> {
        let config = absolute_env("XDG_CONFIG_HOME")
            .or_else(|_| absolute_env("HOME").map(|home| home.join(".config")))?;
        Ok(config.join("systemd").join("user"))
    }

    fn systemctl() -> Command {
        let mut command = Command::new("systemctl");
        command.arg("--user");
        command
    }

    /// Quotes one `ExecStart` word: double quotes, with `\`, `"`, `%`, and `$`
    /// escaped the way systemd reads them.
    fn quote(word: &OsStr) -> String {
        let mut quoted = String::from("\"");
        for character in String::from_utf8_lossy(word.as_bytes()).chars() {
            match character {
                '\\' | '"' => {
                    quoted.push('\\');
                    quoted.push(character);
                }
                '%' => quoted.push_str("%%"),
                '$' => quoted.push_str("$$"),
                _ => quoted.push(character),
            }
        }
        quoted.push('"');
        quoted
    }

    pub(super) fn install(
        service: &Service,
        socket: &Path,
        program: &Path,
        args: &[OsString],
    ) -> Result<(), InstallError> {
        let label = service.label();
        let mut exec = quote(program.as_os_str());
        for arg in args {
            exec.push(' ');
            exec.push_str(&quote(arg));
        }
        let socket_unit = formatdoc! {"
            [Unit]
            Description={label} socket

            [Socket]
            ListenStream={socket}
            SocketMode=0600
            DirectoryMode=0700
            Accept=no
            TriggerLimitIntervalSec=2
            TriggerLimitBurst=200

            [Install]
            WantedBy=sockets.target
        ", socket = socket.display()};
        let service_unit = formatdoc! {"
            [Unit]
            Description={label}
            Requires={label}.socket
            StartLimitIntervalSec=10
            StartLimitBurst=100

            [Service]
            ExecStart={exec}
            StandardInput=socket
            StandardOutput=journal
            StandardError=journal
        "};
        let units = unit_dir()?;
        std::fs::create_dir_all(&units)?;
        std::fs::write(units.join(format!("{label}.socket")), socket_unit)?;
        std::fs::write(units.join(format!("{label}.service")), service_unit)?;
        run(systemctl().arg("daemon-reload"))?;
        run(systemctl()
            .arg("enable")
            .arg("--now")
            .arg(format!("{label}.socket")))
    }

    pub(super) fn uninstall(service: &Service) -> Result<(), InstallError> {
        let label = service.label();
        let units = unit_dir()?;
        let socket_unit = units.join(format!("{label}.socket"));
        if socket_unit.exists() {
            run(systemctl()
                .arg("disable")
                .arg("--now")
                .arg(format!("{label}.socket"))
                .arg(format!("{label}.service")))?;
        }
        remove_if_present(&socket_unit)?;
        remove_if_present(&units.join(format!("{label}.service")))?;
        remove_if_present(&service.socket_path()?)?;
        run(systemctl().arg("daemon-reload"))
    }
}
