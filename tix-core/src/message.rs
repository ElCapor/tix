#[repr(u32)]
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum MessageType {
    Command = 0x1,
    Response = 0x2,
}

impl From<u32> for MessageType {
    fn from(value: u32) -> Self {
        match value {
            0x1 => MessageType::Command,
            0x2 => MessageType::Response,
            _ => panic!("Invalid MessageType value"),
        }
    }
}

#[repr(u64)]
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Command {
    Ping = 0x1,
    HelloWorld = 0x2,
    ShellExecute = 0x3,
    Copy = 0x4,
    ListDrives = 0x5,
    ListDir = 0x6,
    Upload = 0x7,
    Download = 0x8,
    SystemAction = 0x9,
}

impl From<u64> for Command {
    fn from(value: u64) -> Self {
        match value {
            0x1 => Command::Ping,
            0x2 => Command::HelloWorld,
            0x3 => Command::ShellExecute,
            0x4 => Command::Copy,
            0x5 => Command::ListDrives,
            0x6 => Command::ListDir,
            0x7 => Command::Upload,
            0x8 => Command::Download,
            0x9 => Command::SystemAction,
            _ => panic!("Invalid Command value"),
        }
    }
}
