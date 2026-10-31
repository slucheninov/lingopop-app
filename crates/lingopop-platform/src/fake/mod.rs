//! In-memory fakes of the platform traits (always compiled, used by tests here and in later phases).

mod sleeper;

pub use sleeper::FakeSleeper;
