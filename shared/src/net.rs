//! Network configuration shared by the client and server.
//!
//! Centralizes the values both sides must agree on (protocol id, socket ids,
//! ports) plus the deployment endpoints, so they live in one place instead of
//! being duplicated as literals across the two `main.rs` files.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Netcode protocol id. The client and server must use the same value.
pub const PROTOCOL_ID: u64 = 0;

/// Maximum number of simultaneously connected clients.
pub const MAX_CLIENTS: usize = 8;

/// renet2 socket ids. The server registers one socket per id (UDP first, then
/// WebSocket); each client sets `socket_id` to the transport it connects with.
pub const UDP_SOCKET_ID: u8 = 0;
pub const WS_SOCKET_ID: u8 = 1;

/// Public UDP port for native desktop clients.
pub const GAME_UDP_PORT: u16 = 8080;
/// Local WebSocket port for browser clients (fronted by a TLS proxy in prod).
pub const GAME_WS_PORT: u16 = 8081;

/// Deployment server IP — native desktop clients connect here over UDP.
pub const DEFAULT_SERVER_IP: Ipv4Addr = Ipv4Addr::new(158, 180, 62, 178);

/// Deployment domain — browser clients connect here over `wss://` (port 443,
/// terminated by Caddy). A domain is required for a browser-trusted TLS cert.
pub const DEPLOY_SERVER_DOMAIN: &str = "158-180-62-178.sslip.io";

/// Default address a native desktop client connects to.
pub const DEFAULT_SERVER_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(DEFAULT_SERVER_IP), GAME_UDP_PORT);

/// Default address the server binds its UDP socket to.
pub const DEFAULT_BIND_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), GAME_UDP_PORT);

/// Localhost WebSocket endpoint used by dev (non-`wss`) browser builds.
pub const LOCAL_WS_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), GAME_WS_PORT);
