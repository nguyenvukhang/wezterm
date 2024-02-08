use chrono::prelude::*;
use std::sync::Mutex;
use termwiz::input::{InputEvent, KeyCode, KeyEvent};
use termwiz::lineedit::*;

lazy_static::lazy_static! {
    static ref LATEST_LOG_ENTRY: Mutex<Option<DateTime<Local>>> = Mutex::new(None);
}

struct LuaReplHost {
    history: BasicHistory,
    lua: mlua::Lua,
}

fn format_lua_err(err: mlua::Error) -> String {
    match err {
        mlua::Error::SyntaxError {
            incomplete_input: true,
            ..
        } => "...".to_string(),
        _ => format!("{:#}", err),
    }
}

fn fragment_to_expr_or_statement(lua: &mlua::Lua, text: &str) -> Result<String, String> {
    let expr = format!("return {};", text);

    let chunk = lua.load(&expr).set_name("=repl");
    match chunk.into_function() {
        Ok(_) => {
            // It's an expression
            Ok(text.to_string())
        }
        Err(_) => {
            // Try instead as a statement
            let chunk = lua.load(text).set_name("=repl");
            match chunk.into_function() {
                Ok(_) => Ok(text.to_string()),
                Err(err) => Err(format_lua_err(err)),
            }
        }
    }
}

impl LineEditorHost for LuaReplHost {
    fn history(&mut self) -> &mut dyn History {
        &mut self.history
    }

    fn resolve_action(
        &mut self,
        event: &InputEvent,
        editor: &mut LineEditor<'_>,
    ) -> Option<Action> {
        let (line, _cursor) = editor.get_line_and_cursor();
        if line.is_empty()
            && matches!(
                event,
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Escape,
                    ..
                })
            )
        {
            Some(Action::Cancel)
        } else {
            None
        }
    }

    fn render_preview(&self, line: &str) -> Vec<OutputElement> {
        let mut preview = vec![];

        if let Err(err) = fragment_to_expr_or_statement(&self.lua, line) {
            preview.push(OutputElement::Text(err))
        }

        preview
    }
}
