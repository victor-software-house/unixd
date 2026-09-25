# lifecycle Specification

## Purpose
How a unixd daemon runs under its service manager: the stdin listener, unit install and uninstall, one drain path, and the idle exit.

## Requirements

### Requirement: The listener comes from stdin

The serve entry SHALL change its working directory to `/`, then take the
listening socket from stdin, and SHALL NOT bind, unlink, or `shutdown(2)` the
socket.

#### Scenario: Client directory removed

- **WHEN** a client in a temporary directory triggers the first start, the directory is removed, and a second request arrives
- **THEN** the second request succeeds

#### Scenario: Socket survives idle exit

- **WHEN** the daemon exits after its idle timeout
- **THEN** the socket file still exists and the next connection starts a new daemon

### Requirement: Units install and uninstall idempotently

`install` SHALL write the units for this label, load them with
`launchctl bootstrap gui/<uid>` on macOS or `systemctl --user enable --now
<name>.socket` on Linux, and set the restart limits explicitly. `uninstall`
SHALL unload and remove only this label's files.

#### Scenario: Round trip

- **WHEN** `install` runs twice, a client connects, `uninstall` runs, and `install` runs again
- **THEN** each connection is served and exactly one unit set for the label exists at the end

#### Scenario: Path too long

- **WHEN** the socket path is 120 bytes
- **THEN** `install` fails with `SocketPathTooLong` and writes no file

### Requirement: One drain path

The daemon SHALL stop accepting, wait up to `drain_timeout` for open
connections, abort what remains, and exit 0, for `SIGINT`, `SIGTERM`, a
shutdown request, and the idle timer alike.

#### Scenario: Signal during a request

- **WHEN** `SIGTERM` arrives while a handler is running and `drain_timeout` is 5 s
- **THEN** the handler's response is delivered and the daemon exits 0

#### Scenario: Idle timer waits for requests

- **WHEN** `idle_timeout` is 1 s and one request takes 3 s
- **THEN** the daemon does not exit before that request completes
