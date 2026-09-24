pub const BUILD_GIT_SHA: &str = env!("CHAPTERA_BUILD_GIT_SHA");
pub const BUILD_IDENTITY: &str =
    concat!(env!("CARGO_PKG_VERSION"), "+", env!("CHAPTERA_BUILD_GIT_SHA"));
