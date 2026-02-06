mod header;
mod packet;
mod flags;
mod codec;
mod error;
mod message;
mod network;
mod task;
mod state;

pub use header::TixHeader;
pub use message::Command;
pub use flags::Flag;
pub use message::MessageType;
pub use header::HEADER_LENGTH;
pub use packet::Packet;

// For now
pub type Error = error::Error;

const MAX_FRAME_SIZE: usize = 65536*2*2*2;
const MAX_PAYLOAD_SIZE: usize = 65536*2*2;

pub use codec::TixCodec;

pub use network::Connection;
pub use network::TixConnectionSender;

pub use task::Task;
pub use task::TaskPool;
pub use task::TaskEvent;

pub use network::ConnectionInfo;