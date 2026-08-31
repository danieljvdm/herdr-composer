pub mod catalog;
pub mod config;
pub mod images;
pub mod import;
pub mod process;
pub mod request;
pub mod session;
pub mod storage;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const VERSION: u32 = 1;
