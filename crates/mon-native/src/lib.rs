//! Shared native contracts. Callers retain OS-owned handles and process leases;
//! these policies never grant access from an untrusted path or saved receipt.
pub mod archive;
pub mod durability;
pub mod identity;
pub mod process;
pub mod transport;
