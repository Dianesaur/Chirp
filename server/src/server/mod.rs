mod handler;
mod server;

pub use handler::ChatServer;
pub use server::{
    ClientMessage, CommunicateUser, CommunicationType, Connect, Disconnect, IDInfo, Message, WSData,
};