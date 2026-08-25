//! # Packet Transport Module.
//!
//! Introduces a communication abstraction for packet based communication and some concrete
//! implementations for common communication interfaces. This can be useful for exchanging
//! something like CCSDS space packets over different transport mechanisms.
pub mod serial;
pub mod tcp;
pub mod udp;

/// Generic send error.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// Queue is full.
    #[error("queue is full")]
    QueueFull,
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Other error.
    #[error("other error")]
    Other,
}

/// Generic reception error.
#[derive(Debug, thiserror::Error)]
pub enum ReceiveError {
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Other error.
    #[error("other error")]
    Other,
}

/// Generic packet transport trait.
///
/// This abstraction allows different transport mechanism for packetized data like CCSDS space
/// packets.
pub trait PacketTransport {
    /// Send a packet.
    fn send(&mut self, packet: &[u8]) -> Result<(), SendError>;

    /// Receivd packets.
    ///
    /// For each received packet, the closure will be called with the packet as an argument.
    /// The function will return the number of received packets.
    fn receive<F: FnMut(&[u8])>(&mut self, f: F) -> Result<usize, ReceiveError>;

    /// Close the connection, used for graceful shutdowns.
    fn close(&mut self);
}
