//! Раздел «О программе»: версия и тексты лицензий, встроенные при сборке (`build.rs`).

include!(concat!(env!("OUT_DIR"), "/notices.rs"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
