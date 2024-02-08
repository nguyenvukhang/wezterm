use config::lua::mlua;

pub mod guiwin;

fn luaerr(err: anyhow::Error) -> mlua::Error {
    mlua::Error::external(err)
}
