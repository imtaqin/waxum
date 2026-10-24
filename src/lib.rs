//! Waxum library surface. Re-exports the top-level modules so integration
//! tests under `tests/` can build the same router and state the binary
//! uses. The binary in `src/main.rs` also depends on these modules.

#![recursion_limit = "256"]

/// Builds a value of a `#[non_exhaustive]` waproto/whatsapp-rust type from
/// struct-literal syntax. Those types cannot be written as a struct
/// expression outside their crate, even with `..Default::default()`, so this
/// expands to `Default::default()` followed by one assignment per listed
/// field. A trailing `..Default::default()` is accepted and ignored.
macro_rules! nx {
    ($ty:path { $($body:tt)* }) => {{
        let mut v = <$ty as ::std::default::Default>::default();
        nx!(@fields v; $($body)*);
        v
    }};
    (@fields $v:ident;) => {};
    (@fields $v:ident; ..Default::default()) => {};
    (@fields $v:ident; $field:ident : $val:expr, $($rest:tt)*) => {
        $v.$field = $val;
        nx!(@fields $v; $($rest)*);
    };
    (@fields $v:ident; $field:ident : $val:expr) => {
        $v.$field = $val;
    };
    (@fields $v:ident; $field:ident, $($rest:tt)*) => {
        $v.$field = $field;
        nx!(@fields $v; $($rest)*);
    };
    (@fields $v:ident; $field:ident) => {
        $v.$field = $field;
    };
}

pub mod api;
pub mod bootstrap;
pub mod cloud;
pub mod console;
pub mod db;
pub mod device_props;
pub mod embedded;
pub mod error;
pub mod link_preview;
pub mod metrics;
pub mod middleware;
pub mod nats;
pub mod net;
pub mod net_guard;
pub mod preflight;
pub mod state;
pub mod storage;
pub mod workers;
