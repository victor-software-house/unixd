# Activation

## ADDED Requirements

### Requirement: The service manager starts the daemon

unixd SHALL be started only by the platform service manager, on the first
connection to its socket, and SHALL NOT start itself.

#### Scenario: First connection on macOS

- **WHEN** the LaunchAgent is bootstrapped with `launchctl bootstrap gui/$(id -u)` and no daemon process is running
- **AND** a client connects to the socket and sends one request
- **THEN** launchd starts the daemon and the client receives a response

#### Scenario: First connection on Linux

- **WHEN** the socket unit is started with `systemctl --user enable --now unixd-example.socket` and the service is inactive
- **AND** a client connects to the socket and sends one request
- **THEN** systemd starts the service and the client receives a response

### Requirement: The socket arrives on standard input

unixd SHALL take its listening socket from standard input with safe standard
library calls, and SHALL NOT use `unsafe` to adopt a descriptor.

#### Scenario: Adopting the listener

- **WHEN** the daemon starts under launchd with `inetdCompatibility` `Wait = true`, or under systemd with `StandardInput=socket`
- **THEN** `stdin().as_fd().try_clone_to_owned()` yields a descriptor that converts to a listening `UnixListener`

### Requirement: Idle exit keeps the socket

unixd SHALL stop accepting before an idle exit, SHALL NOT unlink the socket
path, and the next connection SHALL start a new daemon.

#### Scenario: Relaunch after idle

- **WHEN** the daemon exits after an idle timeout of 5 seconds
- **AND** a client connects 15 seconds later
- **THEN** the connection is not refused, and a new daemon process answers it
