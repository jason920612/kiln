//! What running Kiln as a dedicated server needs around the game itself: `server.properties`,
//! `eula.txt`, and (later modules) the server list, remote administration and logging.

pub mod eula;
pub mod jar;
pub mod javaprops;
pub mod launch;
pub mod logging;
pub mod mgmt;
pub mod properties;
pub mod query;
pub mod rcon;
pub mod watchdog;
