#[cfg(not(feature = "readline"))]
mod raw;
#[cfg(feature = "readline")]
mod readline;

#[cfg(not(feature = "readline"))]
pub use raw::*;
#[cfg(feature = "readline")]
pub use readline::*;
