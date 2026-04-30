pub mod admin;
pub mod install;
pub mod pin;
pub mod query;
pub mod run;
pub mod shell;

pub(crate) fn http() -> anyhow::Result<qusp_core::effects::LiveHttp> {
    qusp_core::effects::LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))
}
