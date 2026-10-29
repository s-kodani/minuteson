//! Shared recording contracts. OS acquisition and callback ownership stay with
//! native drivers; these decisions are independent of Core Audio and WASAPI.
pub mod catalog;
pub mod commit;
pub mod limits;
pub mod request;
pub mod supervisor;
