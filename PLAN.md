# TIX - AIO Command-and-Control Framework
A high-performance command-and-control framework for dedicated two-machine peer-to-peer connections over direct RJ-45 Ethernet.

## Features

- **File Control** - Bidirectional file transfer with delta-sync, Blake3 integrity verification, and zstd compression

- **Shell Command** - Full PTY support with command history, exit code propagation, and UTF-8 sanitization

- **TixRP (Remote Protocol)** - Low-latency screen capture via DXGI with input injection support

- **TixUpdate (Update Protocol)** - Update slave program

- **TCP Transport** - Secure, low-latency transport layer

## Architecture

### General Layout

 Main PC                          Test PC          
                                                   
┌───────────────┐                ┌───────────────┐ 
│  Tix Master   │◄──────────────►│   Tix Slave   │ 
│               │ RJ45 Wire      │               │ 
└───────────────┘                └───────────────┘ 
                                                   
### Library Layout

                ┌──────────┐                
       ┌────────┤ Tix Core ├──────────┐     
       │        └────┬─────┘          │     
       │             │                │     
       ▼             ▼                ▼     
 ┌───────────┐    ┌──────┐       ┌─────────┐
 │Crypto/ZSTD│    │Errors│       │Protocols│
 └───────────┘    └──────┘       └─────────┘

### Tix-CLI Layout
Tix CLI features simple things => file transfer, shell commands, auto update (for bootstrapping)


### To be defined
-> transports
-> how to execute shell commands over a network like this ?
-> how to send files
-> to what extent do we need crypto


# Use Case
2 computers connected with an rj45 cable.
Remotely control slave computer from master computer, duplicating it's screen to fast it feels like im on the pc physically.


### Requirements (Actual)

- Long lasting tcp connection between master and slave
- Master listens on a port for incoming connections


# Master
-> open a server socket
-> accept a connection
-> handle outgoing commands => file transfer, shell commands, auto update
-> handle incoming responses => file transfer, shell commands, auto update


# Slave
-> connect to master
-> wait for incoming commands => file transfer, shell commands, auto update
-> send back outgoing responses => file transfer, shell commands, auto update

-> if no master connection, close program
-> on master connection lost, try to reconnect, if failed report error and close

# Communication
Communication always happen in this way :

Master -> Slave : Command
Slave  -> Master : Response

Slave can not send commands to master, only responses.
It should be possible to send commands while processing responses, i.e have N shell commands running.

# Framing
## Command
-> 4 Byte header (TIX0, basically TIX{{PROTOCOL_VERSION})
-> 4 Byte checksum (Blake3)
-> 4 Byte message type (Command Or Response)
-> 8 Byte flags (i.e COMPRESSED)
-> 8 Byte command id (if it's a response, then it's the id of the command it responds to)
-> 8 Byte request id (unique id for each command, used to match responses to commands)
-> 8 Byte payload length
-> payload is the actual message data
-> we have an extreme bandwith so compression is useless (100MB/s)


# TixTask
A tix task is a standalone unit of work that performs a task.

i.e 

```rust

// Imagine you're slave and you recieved a command from server
let packet; // parsed packet from server

match packet.get_command() {
    Command::Ping => {
        // send pong response
        let _ = TixTask::spawn(|conn, packet|{
            let pong = Packet::new_response(packet.get_request_id(), Command::Ping, Vec::new());
            if let Err(e) = pong {
                eprintln!("Failed to create pong packet: {}", e);
                return;
            }
            conn.send(pong).unwrap();
        });
    }

    Command::ShellCommand => {
        // handle shell command
        let _ = TixTask::spawn(|conn, packet|{
            // handle shell command
            let command = String::from_utf8_lossy(&packet.get_payload());
            let output = tokio::process::Command::new("cmd")
                .arg("/C")
                .arg(command.as_ref())
                .output()
                .await;
            if let Err(e) = output {
                eprintln!("Failed to execute shell command: {}", e);
                return;
            }
            let stdout = String::from_utf8_lossy(&output.unwrap().stdout);
            let stderr = String::from_utf8_lossy(&output.unwrap().stderr);
            let response = format!("stdout: {}\nstderr: {}", stdout, stderr);
            let response = response.into_bytes();
            let pong = Packet::new_response(packet.get_request_id(), Command::ShellCommand, response);
            if let Err(e) = pong {
                eprintln!("Failed to create pong packet: {}", e);
                return;
            }
            conn.send(pong).unwrap();
        });
    }
}

```