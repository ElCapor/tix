# AGENTS.md - TIX Rust Expert Agent Guidelines

This document transforms any Agentic AI into a senior Rust developer (10+ years experience) with expertise in:
- **Rust best practices and patterns**
- **Tokio async runtime mastery**
- **High-performance networking (TCP/UDP)**
- **Remote desktop protocol implementation**
- **Optimized networked Rust applications**

---

## Core Philosophy

### Rust Excellence Principles

1. **Zero-Cost Abstractions**: Every abstraction must be justified by real performance requirements. Remote desktop protocols demand microsecond-level latency, so measure before optimizing.

2. **Fearless Concurrency**: Leverage Rust's type system to eliminate data races at compile time. Use `Send`, `Sync`, and lifetime annotations to encode correctness guarantees.

3. **Ownership-Driven Design**: Model your protocols around ownership transfer. Packets own their payloads, connections own their state, and resources clean themselves up via RAII.

4. **Result-Based Error Handling**: Use `Result<T, E>` for all fallible operations. Propagate errors through the call stack with `?` operator. Define domain-specific error types that enable intelligent recovery.

5. **Testing at All Levels**:
   - Unit tests for pure functions and protocol logic
   - Integration tests for connection handling
   - Property-based tests for codec correctness
   - Load tests for performance validation

### Project Structure Guidelines

```
tix/
├── AGENTS.md                    # This file
├── Cargo.toml                   # Workspace manifest
├── tix-core/                    # Core protocol library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              # Public API exports
│       ├── error.rs            # Error types
│       ├── packet.rs           # Packet structures
│       ├── header.rs           # Frame header
│       ├── codec/              # Encoding/decoding
│       ├── network/            # Connection management
│       ├── state/              # State machines
│       └── task/               # Task orchestration
├── tix-master/                 # Master (controller) application
└── tix-slave/                  # Slave (target) application
```

---

## Tokio Async Patterns

### Runtime Selection and Configuration

**For TIX's RJ45 direct connection use case:**

```rust
// tokio.toml configuration
[dependencies]
tokio = { version = "1", features = [
    "full",          # All features for development
    "tracing",       # Structured logging
] }

[profile.release]
lto = true           # Link-time optimization
codegen-units = 1    # Maximum optimization
opt-level = 3        # Maximum optimization
panic = "abort"      # Smaller binary, no unwinding overhead
```

**Runtime configuration for low-latency networking:**

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Current thread runtime has lowest overhead for single-connection scenarios
    // Ideal for direct RJ45 where only one peer connection exists
}
```

**Multi-threaded runtime for scalability:**

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    // Use multi-threaded when:
    // - Multiple concurrent connections
    // - CPU-intensive work (video encoding, compression)
    // - Blocking operations that shouldn't monopolize async tasks
}
```

### Task Spawning Patterns

**Pattern 1: Spawn-per-connection with owned state**

```rust
// For master: accept connections and spawn handler tasks
async fn accept_connections(listener: &TcpListener) -> Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            handle_connection(stream, addr).await;
        });
    }
}

// Handler owns its connection state
async fn handle_connection(stream: TcpStream, addr: SocketAddr) {
    let mut connection = Connection::new(stream);
    // Connection state is owned, no borrowing issues
}
```

**Pattern 2: Actor pattern with message passing**

```rust
// For complex state management, use channels as actor boundaries
struct SlaveActor {
    receiver: Receiver<ActorMessage>,
    state: SlaveState,
}

impl SlaveActor {
    async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.handle_message(msg).await;
        }
    }
}
```

**Pattern 3: Select for timeout and cancellation**

```rust
use tokio::time::{timeout, Duration};

async fn send_with_timeout(conn: &Connection, packet: Packet) -> Result<Packet> {
    let timeout_duration = Duration::from_secs(5);
    
    timeout(timeout_duration, conn.send(packet)).await??;
    timeout(timeout_duration, conn.receive()).await??
}
```

### Async I/O Best Practices

**Buffer management for low latency:**

```rust
// Read buffer should be appropriately sized
const READ_BUFFER_SIZE: usize = 64 * 1024; // 64KB for screen captures

// Use BufReader for protocols with framing
let stream = BufReader::new(TcpStream::connect(addr).await?);
```

**Framed streaming with LengthDelimitedCodec:**

```rust
use tokio_util::codec::{LengthDelimitedCodec, Framed};

let codec = LengthDelimitedCodec::builder()
    .max_frame_length(10 * 1024 * 1024) // 10MB max for large payloads
    .length_field_length(4)
    .new_codec();

let mut framed = Framed::new(stream, codec);
```

---

## Networking Patterns

### TCP Master-Slave Architecture

**Connection lifecycle for TIX:**

```
Master (Listener)                    Slave (Connector)
    │                                      │
    │  1. Listen on port                   │
    │◄─────────────────────────────────────│
    │                    2. Connect        │
    │                                      │
    │  3. Handshake (version negotiation)  │
    │◄────────────────────────────────────►│
    │                                      │
    │  4. Command →                        │
    │◄─ Response                           │
    │  (concurrent, multiple in flight)     │
    │                                      │
    │  5. Heartbeat (keep-alive)           │
    │◄────────────────────────────────────►│
    │                                      │
    │  6. Disconnect                        │
```

**TCP connection establishment:**

```rust
// Master side - listening
async fn start_master(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    
    // Set socket options for low latency
    let stream = listener.accept().await?.0;
    set_socket_options(&stream)?;
    
    Ok(listener)
}

// Slave side - connecting with retry
async fn connect_to_master(addr: SocketAddr, max_retries: u32) -> Result<TcpStream> {
    let mut attempts = 0;
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                set_socket_options(&stream)?;
                return Ok(stream);
            }
            Err(e) if attempts < max_retries => {
                attempts += 1;
                tokio::time::sleep(Duration::from_secs(1 << attempts)).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn set_socket_options(stream: &TcpStream) -> Result<()> {
    // Disable Nagle's algorithm for low latency
    stream.set_nodelay(true)?;
    
    // Set keep-alive to detect dead connections
    stream.set_keepalive(Some(Duration::from_secs(30)))?;
    
    Ok(())
}
```

**TCP keep-alive and heartbeat:**

```rust
// Implement application-level heartbeat since TCP keep-alive 
// may take hours to trigger on some platforms

struct Heartbeat {
    last_ping: Instant,
    interval: Duration,
}

impl Heartbeat {
    fn new() -> Self {
        Self {
            last_ping: Instant::now(),
            interval: Duration::from_secs(30),
        }
    }
    
    async fn tick(&mut self, conn: &mut Connection) -> Result<()> {
        if self.last_ping.elapsed() > self.interval {
            conn.send(Packet::ping()).await?;
            self.last_ping = Instant::now();
        }
        Ok(())
    }
}

// Use select! to handle both data and heartbeat
tokio::select! {
    result = connection.receive() => {
        // Handle incoming packet
    }
    _ = heartbeat.tick(&mut connection) => {
        // Send heartbeat
    }
}
```

### UDP for Low-Latency Scenarios

**When to use UDP in remote desktop:**
- Real-time screen updates (loss tolerance)
- Mouse/keyboard input (loss acceptable, latency critical)
- Audio streaming

**UDP implementation pattern:**

```rust
use tokio::net::UdpSocket;
use std::net::SocketAddr;

struct UdpChannel {
    socket: UdpSocket,
    remote_addr: SocketAddr,
    sequence: u32,
}

impl UdpChannel {
    async fn send(&mut self, data: &[u8]) -> Result<usize> {
        let packet = UdpPacket::wrap(data, self.sequence);
        self.sequence += 1;
        self.socket.send_to(&packet, self.remote_addr).await?;
        Ok(packet.len())
    }
    
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let (len, _) = self.socket.recv_from(buffer).await?;
        Ok(len)
    }
}

// Reliability layer for UDP
struct ReliableUdp {
    channel: UdpChannel,
    acks: HashMap<u32, Instant>, // Pending acknowledgments
    window_size: usize,
}
```

**RTP-like packet structure for screen data:**

```rust
#[derive(Debug)]
struct RtpPacket {
    sequence: u16,
    timestamp: u32,
    marker: bool,      // Marks frame boundary
    payload_type: u8,
    payload: Vec<u8>,
}

impl RtpPacket {
    fn encode_frame_boundary(frame_id: u32) -> u32 {
        // Marks end of a video frame for reconstruction
        (frame_id << 1) | 1
    }
}
```

### Protocol Framing

**TIX packet header structure:**

```rust
// Must be exactly as specified in PLAN.md
#[repr(C, packed)]
struct PacketHeader {
    magic: [u8; 4],      // b'TIX0' or b'TIX1'
    checksum: [u8; 32],  // Blake3 hash
    message_type: u32,    // Command or Response
    flags: u64,          // Bitmask for compression, encryption, etc.
    request_id: u64,     // Unique command identifier
    payload_length: u64, // Size of following payload
}

impl PacketHeader {
    const MAGIC_TIX0: [u8; 4] = *b"TIX0";
    const SIZE: usize = std::mem::size_of::<PacketHeader>();
}
```

**Codec implementation:**

```rust
// Length-prefixed codec for variable-length payloads
struct TixCodec {
    next_length: Option<usize>,
}

impl Decoder for TixCodec {
    type Item = Packet;
    type Error = TixError;
    
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Packet>, Self::Error> {
        if let Some(len) = self.next_length {
            if buf.len() < len {
                return Ok(None);
            }
            let data = buf.split_to(len);
            self.next_length = None;
            return Packet::decode(&data);
        }
        
        if buf.len() < PacketHeader::SIZE {
            return Ok(None);
        }
        
        let header = PacketHeader::decode(&buf[..PacketHeader::SIZE])?;
        let total_len = PacketHeader::SIZE + header.payload_length as usize;
        
        if buf.len() < total_len {
            self.next_length = Some(total_len);
            return Ok(None);
        }
        
        buf.advance(PacketHeader::SIZE);
        let payload = buf.split_to(header.payload_length as usize);
        
        Packet::from_header_and_payload(header, payload)
    }
}

impl Encoder<Packet> for TixCodec {
    fn encode(&mut self, item: Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = item.encode()?;
        dst.extend_from_slice(&encoded);
        Ok(())
    }
}
```

---

## Remote Desktop Protocol Implementation

### TixRP - Screen Capture Protocol

**Architecture overview:**

```
┌─────────────────────────────────────────┐
│           Screen Capture Pipeline        │
├─────────────────────────────────────────┤
│  DXGI Output → Frame Copy → Encoding → │
│  Network Send → Network Recv → Decode  │
│         → Display on Master            │
└─────────────────────────────────────────┘
```

**Screen capture with DXGI (Windows):**

```rust
// For Windows screen capture, use DXGI Desktop Duplication API
// This provides the lowest latency capture method

use windows::{
    core::*, 
    Win32::Graphics::Dxgi::*,
    Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D11::*,
};

struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    texture: ID3D11Texture2D,
}

impl DxgiCapturer {
    async fn capture_frame(&mut self) -> Result<ScreenFrame> {
        // Acquire next frame
        let frame_info = self.acquire_frame()?;
        
        // Copy to CPU-accessible texture
        self.context.copy_resource(&self.texture, &frame_info.acquired_desktop_image);
        
        // Map for CPU access
        let mapped = self.map_texture()?;
        
        // Encode and send
        let encoded = self.encode_frame(&mapped);
        self.release_frame();
        
        Ok(encoded)
    }
}
```

**Screen data encoding for network transfer:**

```rust
// For screen data, use efficient compression
// Zstandard for general compression, but consider:
// - Per-row delta encoding for minor changes
// - Lossy compression for high-motion areas
// - Resolution scaling during high bandwidth

struct ScreenEncoder {
    compressor: zstd::stream::Encoder<'static, Vec<u8>>,
    frame_id: u32,
}

impl ScreenEncoder {
    fn encode(&mut self, frame: &ScreenFrame) -> Result<EncodedFrame> {
        // Detect changes from previous frame
        let diff = frame.diff_from(&self.previous_frame);
        
        // Compress only changed regions
        let compressed = self.compressor.compress(&diff)?;
        
        self.previous_frame = frame.clone();
        self.frame_id += 1;
        
        Ok(EncodedFrame {
            frame_id: self.frame_id,
            data: compressed,
            timestamp: Instant::now(),
        })
    }
}
```

**Input injection from master to slave:**

```rust
// Inject mouse/keyboard events to the slave system
struct InputInjector;

impl InputInjector {
    async fn inject_mouse(&self, event: MouseEvent) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::Win32::System::Console::*;
        
        // Send INPUT structures to the target window
        let input = INPUT {
            type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: event.x,
                    dy: event.y,
                    mouseData: 0,
                    dwFlags: event.event_type.to_flags(),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
        Ok(())
    }
    
    async fn inject_keyboard(&self, event: KeyEvent) -> Result<()> {
        let flags = match event.pressed {
            true => KEYEVENTF_SCANCODE | KEYEVENTF_KEYDOWN,
            false => KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP,
        };
        
        let input = INPUT {
            type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: event.virtual_key_code,
                    wScan: event.scan_code,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
        Ok(())
    }
}
```

### Frame Rate and Latency Optimization

**Adaptive quality based on bandwidth:**

```rust
struct AdaptiveEncoder {
    target_latency: Duration,
    current_bitrate: u32,
    quality: u8,
    bandwidth_estimator: BandwidthEstimator,
}

impl AdaptiveEncoder {
    async fn adjust_quality(&mut self) {
        let estimated_bandwidth = self.bandwidth_estimator.estimate().await;
        let latency = self.bandwidth_estimator.latency();
        
        if latency > self.target_latency * 2 {
            // Latency too high, reduce quality
            self.quality = self.quality.saturating_sub(1);
            self.current_bitrate = (self.current_bitrate * 90) / 100;
        } else if estimated_bandwidth > self.current_bitrate * 110 / 100 {
            // Can increase quality
            self.quality = (self.quality + 1).min(100);
            self.current_bitrate = (self.current_bitwidth * 110) / 100;
        }
    }
}
```

---

## File Transfer Protocol

### Bidirectional File Transfer with Delta-Sync

**File transfer architecture:**

```rust
struct FileTransfer {
    block_size: usize,
    hash_algo: blake3::Hasher,
}

impl FileTransfer {
    // For RJ45 direct connection, bandwidth is ~100MB/s
    // Compression may not help, but integrity verification is critical
    
    async fn send_file(&self, path: &Path, conn: &mut Connection) -> Result<FileMetadata> {
        let file = tokio::fs::File::open(path).await?;
        let metadata = file.metadata().await?;
        
        // Send metadata first
        let meta = FileMetadata {
            path: path.to_string_lossy(),
            size: metadata.len(),
            modified: metadata.modified()?,
        };
        conn.send(Packet::file_metadata(&meta)).await?;
        
        // Stream file in chunks with Blake3 verification
        let mut hasher = blake3::Hasher::new();
        let mut reader = BufReader::new(file);
        let mut buffer = vec![0u8; self.block_size];
        let mut offset = 0u64;
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            
            hasher.update(&buffer[..n]);
            let chunk = FileChunk {
                offset,
                data: &buffer[..n],
            };
            conn.send(Packet::file_chunk(&chunk)).await?;
            offset += n as u64;
        }
        
        // Send final hash for verification
        let final_hash = hasher.finalize();
        conn.send(Packet::file_hash(&final_hash)).await?;
        
        Ok(meta)
    }
    
    async fn receive_file(&self, conn: &mut Connection, output: &Path) -> Result<()> {
        // Receive metadata
        let meta: FileMetadata = conn.receive().await?;
        
        let mut file = tokio::fs::File::create(output).await?;
        let mut hasher = blake3::Hasher::new();
        let mut received_hash = None;
        
        while let Ok(packet) = conn.receive().await {
            match packet.type_() {
                PacketType::FileChunk => {
                    let chunk = packet.into_chunk()?;
                    hasher.update(&chunk.data);
                    file.write_all(&chunk.data).await?;
                }
                PacketType::FileHash => {
                    received_hash = Some(packet.into_hash()?);
                    break;
                }
                _ => return Err(TixError::UnexpectedPacket),
            }
        }
        
        // Verify integrity
        let computed_hash = hasher.finalize();
        if Some(&computed_hash) != received_hash.as_ref() {
            return Err(TixError::FileIntegrityFailed);
        }
        
        Ok(())
    }
}
```

**Delta-sync for modified files:**

```rust
struct DeltaSync {
    chunk_size: u64,
}

impl DeltaSync {
    async fn compute_delta(&self, old: &Path, new: &Path) -> Result<DeltaFile> {
        // Use bsdiff for binary diffs
        // For screen captures, use per-row comparison
        
        let old_chunks = self.chunk_file(old).await?;
        let new_chunks = self.chunk_file(new).await?;
        
        // Find matching chunks (content-addressable)
        let old_chunk_hashes: HashMap<Hash, u64> = old_chunks
            .iter()
            .map(|c| (c.hash, c.offset))
            .collect();
        
        let mut operations = Vec::new();
        let mut i = 0;
        
        for new_chunk in &new_chunks {
            if let Some(&old_offset) = old_chunk_hashes.get(&new_chunk.hash) {
                // Chunk already exists in old file
                operations.push(DeltaOp::Copy {
                    from_offset: old_offset,
                    to_offset: new_chunk.offset,
                    length: new_chunk.length,
                });
            } else {
                // New chunk, send actual data
                operations.push(DeltaOp::Write {
                    offset: new_chunk.offset,
                    data: new_chunk.data.to_vec(),
                });
            }
        }
        
        Ok(DeltaFile { operations })
    }
}
```

---

## Shell Command Execution

### PTY-Based Command Execution

**Full PTY support with proper cleanup:**

```rust
use tokio::process::{Command, Child};
use std::os::windows::process::CommandExt;

struct ShellSession {
    process: Child,
    stdout: BufReader<ChildStdout>,
    stdin: BufWriter<ChildStdin>,
    history: Vec<String>,
}

impl ShellSession {
    async fn start() -> Result<Self> {
        let mut cmd = Command::new("cmd.exe");
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP)
           .stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped())
           .kill_on_drop(true);
        
        let mut child = cmd.spawn()?;
        
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr = BufReader::new(child.stderr.take().unwrap());
        
        Ok(ShellSession {
            process: child,
            stdin,
            stdout,
            history: Vec::new(),
        })
    }
    
    async fn execute(&mut self, command: &str) -> Result<CommandOutput> {
        // Add to history
        self.history.push(command.to_string());
        
        // Send command with newline
        self.stdin.write_all(command.as_bytes()).await?;
        self.stdin.write_all(b"\r\n").await?;
        self.stdin.flush().await?;
        
        // Read output until prompt appears
        let mut output = String::new();
        let mut buffer = [0u8; 4096];
        
        loop {
            let n = self.stdout.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            output.push_str(&String::from_utf8_lossy(&buffer[..n]));
            
            // Check for prompt (cmd.exe typically shows current directory)
            if output.contains(">") {
                break;
            }
        }
        
        Ok(CommandOutput {
            stdout: output,
            exit_code: self.process.wait().await?.code().unwrap_or(-1),
        })
    }
}
```

**UTF-8 sanitization:**

```rust
fn sanitize_output(data: &[u8]) -> String {
    String::from_utf8_lossy(data)
        .replace('\0', "")  // Remove null bytes
        .replace('\r', "") // Normalize line endings
        .trim_end()         // Remove trailing whitespace
        .to_string()
}
```

---

## Error Handling Patterns

### Domain-Specific Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum TixError {
    #[error("Connection lost: {0}")]
    ConnectionLost(#[from] std::io::Error),
    
    #[error("Protocol violation: {0}")]
    ProtocolViolation(&'static str),
    
    #[error("Invalid packet: {0}")]
    InvalidPacket(&'static str),
    
    #[error("Timeout after {0:?}")]
    Timeout(Duration),
    
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u32),
    
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Encoding error: {0}")]
    Encoding(#[from] bincode::Error),
    
    #[error("Compression error: {0}")]
    Compression(#[from] zstd::Error),
}

// Implement From for common error conversions
impl From<TixError> for std::io::Error {
    fn from(e: TixError) -> Self {
        match e {
            TixError::Io(e) => e,
            TixError::ConnectionLost(e) => e,
            _ => std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
        }
    }
}
```

### Retry Logic with Backoff

```rust
struct RetryPolicy {
    max_retries: u32,
    base_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
}

impl RetryPolicy {
    async fn execute<F, T, E>(&self, operation: F) -> Result<T>
    where
        F: Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut attempts = 0;
        let mut delay = self.base_delay;
        
        loop {
            match operation.await {
                Ok(result) => return Ok(result),
                Err(e) if attempts < self.max_retries => {
                    attempts += 1;
                    eprintln!("Attempt {}/{} failed: {}", attempts, self.max_retries, e);
                    
                    tokio::time::sleep(delay).await;
                    delay = (delay.as_secs_f64() * self.multiplier)
                        .min(self.max_delay.as_secs_f64()) as u64;
                    delay = Duration::from_secs(delay);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}
```

---

## Performance Optimization Guidelines

### Memory Management

**Arena allocation for protocol buffers:**

```rust
// Use bumpalo for transient allocations
use bumpalo::{Bump, arena};

#[derive(Debug)]
pub struct PacketArena {
    arena: Bump,
}

impl PacketArena {
    pub fn new() -> Self {
        Self {
            arena: Bump::new(),
        }
    }
    
    pub fn allocate_packet(&self, header: PacketHeader, payload: &[u8]) -> Box<Packet<'static>> {
        let payload_copy = self.arena.alloc_slice_copy(payload);
        Box::new(Packet {
            header,
            payload: payload_copy.into(),
        })
    }
}
```

**Zero-copy parsing with bytes::Bytes:**

```rust
use bytes::Bytes;

struct Packet {
    header: PacketHeader,
    payload: Bytes,  // Zero-copy reference to original buffer
}

impl Packet {
    // Payload is a view into the original received buffer
    // No copying required
    fn payload(&self) -> &[u8] {
        &self.payload
    }
}
```

### Lock-Free Data Structures

**For concurrent access patterns:**

```rust
use crossbeam::atomic::AtomicCell;
use crossbeam::queue::SegQueue;

struct ConcurrentCommandQueue {
    queue: SegQueue<Command>,
    processing: AtomicCell<bool>,
}

impl ConcurrentCommandQueue {
    pub fn push(&self, command: Command) {
        self.queue.push(command);
    }
    
    pub fn try_pop(&self) -> Option<Command> {
        self.queue.try_pop()
    }
}
```

### Profiling and Benchmarking

**Benchmark setup:**

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_packet_encoding(c: &mut Criterion) {
    let packet = create_test_packet();
    
    c.bench_function("packet_encode", |b| {
        b.iter(|| {
            black_box(&packet).encode().unwrap()
        })
    });
}

fn bench_screen_capture(c: &mut Criterion) {
    c.bench_function("screen_capture_1080p", |b| {
        b.iter(|| {
            black_box(capturer).capture_frame().unwrap()
        })
    });
}

criterion_group!(benches, bench_packet_encoding, bench_screen_capture);
criterion_main!(benches);
```

**Memory profiling with dhat:**

```rust
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();
    // Your code here
}
```

---

## Testing Strategies

### Protocol Testing

**Property-based testing with proptest:**

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_packet_roundtrip(packet in any::<Packet>()) {
        let encoded = packet.encode().unwrap();
        let decoded = Packet::decode(&encoded).unwrap();
        prop_assert_eq!(packet, decoded);
    }
    
    #[test]
    fn test_header_checksum(header in any::<PacketHeader>()) {
        let checksum = blake3::hash(&header.as_bytes());
        let mut header_with_checksum = header;
        header_with_checksum.checksum = *checksum.as_bytes();
        assert!(header_with_checksum.verify_checksum());
    }
}
```

**Integration testing:**

```rust
#[tokio::test]
async fn test_full_connection_cycle() {
    let master_addr = start_master().await?;
    let slave = connect_to_master(master_addr).await?;
    
    // Test handshake
    let version = slave.handshake().await?;
    assert_eq!(version, PROTOCOL_VERSION);
    
    // Test command-response
    let response = slave.send_command(Command::Ping).await?;
    assert_eq!(response, Response::Pong);
    
    Ok(())
}
```

---

## Security Considerations

### Secure Protocol Design

**Encryption for non-local connections:**

```rust
// For production use over untrusted networks
struct EncryptedConnection {
    inner: TcpStream,
    encryptor: aead::Encryptor,
    decryptor: aead::Decryptor,
}

impl EncryptedConnection {
    async fn send(&mut self, packet: &Packet) -> Result<()> {
        let plaintext = packet.encode()?;
        let nonce = self.generate_nonce();
        let ciphertext = self.encryptor.encrypt(&nonce, plaintext.as_slice())?;
        self.inner.send(&ciphertext).await?;
        Ok(())
    }
}

// For RJ45 direct cable, encryption may add unnecessary latency
// Use flags to negotiate encryption
```

---

## Summary Checklist

When implementing TIX components, ensure:

- [ ] **Ownership is clear**: Every resource has a clear owner
- [ ] **Errors are typed**: Domain-specific error types, not generic `String`
- [ ] **Async is non-blocking**: Never block in async code
- [ ] **Connections are managed**: Proper lifecycle, cleanup on drop
- [ ] **Protocols are versioned**: Handshake includes version negotiation
- [ ] **Data is validated**: All input is validated at protocol boundaries
- [ ] **Tests exist**: Unit tests, integration tests, property tests
- [ ] **Benchmarks exist**: Performance critical paths are benchmarked
- [ ] **Documentation is clear**: Each public API has documentation
- [ ] **Zero allocations in hot paths**: Profile before optimizing

---

## References

### Tokio Documentation
- [Tokio Runtime Configuration](https://tokio.rs/tokio/tutorial)
- [Async Best Practices](https://tokio.rs/tokio/topics/patterns)

### Rust Books
- [The Rust Programming Language](https://doc.rust-lang.org/book/)
- [Async Rust](https://rust-lang.github.io/async-book/)
- [The Cargo Book](https://doc.rust-lang.org/cargo/)

### Protocol References
- [RDP Protocol Specification](https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/)
- [DXGI Desktop Duplication](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)

### Performance
- [Rust Benchmarking](https://github.com/bheisler/criterion.rs)
- [Profiling Rust](https://nnethercote.github.io/perf-book/)
